//! `winc` 可执行文件的入口：读取配置和状态，按命令行参数或上次的现场打开文件，创建窗口和事件循环。
//!
//! `app` 模块放 `App` 的状态与各项功能，`view` 绘制并把几何交给 `layout`，
//! `keymap`、`theme`、`icons` 给出快捷键、配色和尺寸，`field` 是单行输入框，
//! `macos` 放直接调用 `AppKit` 的能力。Windows 上
//! 是图形界面程序：从命令行启动时接上该命令行的控制台，`--help` 和错误信息输出到该控制台
//! （`windows::attach_console`）。

// 不加时 Windows 每次启动 winc 都会另开一个控制台窗口。
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod askpass;
mod bench;
mod cli;
mod crash;
mod field;
mod frame_log;
mod icons;
mod instance;
mod keymap;
#[cfg(target_os = "macos")]
mod large_alloc;
mod layout;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
mod portal;
mod shell_command;
mod system;
mod theme;
#[cfg(all(unix, not(target_os = "macos")))]
mod trash;
#[cfg(not(target_os = "macos"))]
mod updates;
mod view;
#[cfg(all(unix, not(target_os = "macos")))]
mod wayland;
mod winc;
#[cfg(windows)]
mod windows;

/// 大块内存释放后立即归还给系统。
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOC: large_alloc::LargeAlloc = large_alloc::LargeAlloc;

use std::path::Path;

use winc_core::Lines;

use winc_config::{Config, Session, State};
use winc_editor::Editor;
use winit::event_loop::EventLoop;

use app::App;
use bench::Bench;
use winc::{Envelope, Winc};

fn main() {
    // 作为 `git`、`ssh` 的 askpass 程序启动时不创建窗口。
    if let Some(code) = askpass::helper() {
        std::process::exit(code);
    }
    shell_command::enter_real_path();
    crash::install();
    #[cfg(windows)]
    windows::attach_console();
    // `--help` 和无法识别的选项不需要窗口，先处理参数；用法与错误信息按系统语言输出。
    winc_i18n::set_locale(winc_i18n::Locale::system());
    let args = match cli::parse(std::env::args_os().skip(1)) {
        Ok(args) if args.help => {
            println!("{}", cli::usage());
            return;
        }
        Ok(args) if args.version => {
            println!("winc {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };

    let bench = Bench::from_env();
    bench.on_start();

    // 便携版首次运行时导入用户目录中的配置和状态，须在读取配置之前；测量时不读写状态，不导入。
    if !bench.silent() {
        winc_config::files::import_into_portable();
    }

    // 配置必须在加载字体之前读出，第一帧即使用用户指定的字体和字号。读入配置时的问题按系统语言
    // 描述（语言已在处理参数之前设定），界面语言随后按配置设定。
    let (config, problems) = Config::load();
    set_locale(config.language.unwrap_or_default().resolve());
    for problem in &problems {
        eprintln!("winc: {problem}");
    }

    // 非 UTF-8 文件按内容推测编码；内容很短、特征不足时参考用户的语言（简体中文倾向 GBK）。
    // 必须在打开文件之前设置。
    set_encoding_region();

    // 在创建窗口之前读入文件，使第一帧即可显示内容。
    let mut editor = Editor::default();
    editor.tab_width = config.tab_width.unwrap_or(editor.tab_width);

    // 最近打开的目录和上次的现场。测量时不读也不写：测量对象只能是命令行给出的样本文件，
    // 且样本目录不能进入用户的最近列表。
    let state_path = (!bench.silent()).then(winc_config::state_path).flatten();
    let state = state_path
        .as_deref()
        .map(State::load_from)
        .unwrap_or_default();

    // 未保存内容的备份目录位于状态文件所在目录下；测量时不读也不写。
    let backup_root = state_path
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join("backups"));

    // 用户下载的 PDFium 也放在状态文件所在目录下（便携版在数据目录中）。测量时同样设置：PDF 的
    // 打开方式与平时一致。
    if let Some(dir) = winc_config::state_path().as_deref().and_then(Path::parent) {
        winc_preview::pdf::set_engine_dir(dir.join("pdfium"));
    }

    // 已经有 winc 在运行：交给它打开并切到前台，本进程不再启动。
    // 临时打开（`--wait`、`-`）要等标签关闭或读标准输入，总是自己打开；测量时、指定了
    // `--new-instance` 时也不交出去。
    // 读不了的目标现在就报告：之后可能交给其他进程，错误不再回到这个终端。
    let mut args = args;
    let failed = shell_command::report_unreadable(&mut args);
    if failed && args.targets.is_empty() {
        std::process::exit(1);
    }
    if !args.one_off() && !args.new_instance && !bench.silent() && hand_over(&args) {
        std::process::exit(i32::from(failed));
    }
    // 从终端启动时窗口进程转到后台，命令立即返回。
    if !bench.silent() && shell_command::detach(&args) {
        std::process::exit(i32::from(failed));
    }

    for problem in open_targets(&mut editor, &args) {
        eprintln!("winc: {problem}");
    }
    // `--wait` 给的全是打不开的目标（例如目录）时没有标签可等，窗口在命令行目标处理完之后
    // 不会自行退出，调用 winc 的程序（如 `git`）会一直等待。此时直接以失败状态退出。
    if args.wait && !args.stdin && editor.documents.is_empty() {
        eprintln!(
            "winc: {}",
            winc_i18n::t!("cmd-cli-wait-needs-openable-file")
        );
        std::process::exit(1);
    }
    // 没有命令行目标时恢复上次的现场；没有现场则显示开始页。
    let session = (args.targets.is_empty() && !args.one_off())
        .then(|| state.last_session())
        .flatten()
        .filter(|session| open_session_folder(&mut editor, session));
    // 从命令行打开的目录没有现场，只恢复其未保存的内容；临时打开时不恢复任何内容。
    let restored = if args.one_off() {
        Default::default()
    } else {
        app::restore_at_launch(&mut editor, session, backup_root.as_deref())
    };
    let silent = bench.silent();
    // 上次退出时其他窗口开着的目录：没有命令行目标时与恢复上次的现场一样全部打开，各占一个窗口。
    let others = if args.targets.is_empty() && !args.one_off() {
        state.others.clone()
    } else {
        Vec::new()
    };

    let mut builder = EventLoop::<Envelope>::with_user_event();

    // 测量时静默运行：`Accessory` 策略下应用不出现在程序坞和 ⌘Tab 中，也不占用菜单栏；
    // 不调用 `activateIgnoringOtherApps`，键盘焦点保留在当前前台应用上。
    #[cfg(target_os = "macos")]
    if silent {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder
            .with_activation_policy(ActivationPolicy::Accessory)
            .with_activate_ignoring_other_apps(false)
            .with_default_menu(false);
    }

    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            eprintln!(
                "winc: {}",
                winc_i18n::t!("cmd-cli-event-loop-failed", error = e.to_string())
            );
            std::process::exit(1);
        }
    };

    // 拖到程序坞图标上、由系统转交给 winc 打开的文件和目录。winc 因此被启动时，系统在启动
    // 完成之前就交来这些路径，因此必须在事件循环开始运行之前注册回调；这些事件在窗口建好以后
    // 才处理。测量时不注册，样本文件只能由命令行指定。
    #[cfg(target_os = "macos")]
    if !silent {
        let proxy = event_loop.create_proxy();
        macos::on_open_files(move |paths| {
            let _ = proxy.send_event(Envelope {
                to: winc::Recipient::Active,
                event: app::UserEvent::Opened(paths),
            });
        });
    }

    // 设置页的改动写回该路径；无法计算路径（没有 HOME）时只修改内存中的配置。
    let config_path = winc_config::config_path();
    // 临时打开（`--wait`、`-`）和测量时只有这一个窗口：不记现场、不备份，新窗口没有可记录的状态。
    let template = (!args.one_off() && !silent).then(|| winc::Template {
        config_path: config_path.clone(),
        state_path: state_path.clone(),
        backup_root: backup_root.clone(),
    });
    let mut winc = Winc::new(Some(event_loop.create_proxy()), template);
    let (key, proxy) = winc.reserve();
    let mut app = App::new(editor, proxy, bench, config);
    app.config_path = config_path;
    app.commit.set_flat(!state.commit_tree);
    app.editor.diff_unified = app.config.diff_inline == Some(true);
    app.state = state;
    app.restore_panel_width();
    app.apply_preview_state();
    // 临时打开（`--wait`、`-`）时不记录现场、不加入最近列表、不备份，下次启动的状态不受影响。
    if args.one_off() {
        app.start_one_off(&args);
    } else {
        app.state_path = state_path;
        app.backup.root = backup_root;
        app.adopt_restored(restored);
        // 从命令行打开的目录同样记入「最近打开」。
        app.remember_workspace();
    }

    winc.add(key, app);
    winc.restore_windows(&others);
    // 指定了 `--new-instance` 的进程不接收之后启动的 winc 交来的目录和文件，这些仍交给原来
    // 运行着的 winc。
    if !args.new_instance {
        winc.listen_for_launches();
    }
    if let Err(e) = event_loop.run_app(&mut winc) {
        eprintln!("winc: {e}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    updates::after_exit();
}

/// 按命令行参数打开目录和文件，返回需要提示用户的 `Vec<String>`。
///
/// 给出多个文件时，以第一个文件所在的目录作为项目；`文件:行` 形式的目标会跳到对应行
/// （行号和列号从 0 起）。临时打开（`--wait`、`-`）时不打开项目，只打开给出的文件。
pub(crate) fn open_targets(editor: &mut Editor, args: &cli::Args) -> Vec<String> {
    let mut problems = Vec::new();
    let mut first_file = None;
    for target in &args.targets {
        if target.path.is_dir() {
            if args.one_off() {
                let path = target.path.display().to_string();
                problems.push(winc_i18n::t!("cmd-cli-wait-folder", path = path));
                continue;
            }
            if let Err(e) = editor.open_folder(&target.path) {
                let (path, error) = (target.path.display().to_string(), e.to_string());
                problems.push(winc_i18n::t!(
                    "cmd-cli-open-failed",
                    path = path,
                    error = error
                ));
            }
            continue;
        }
        let index = editor.open(&target.path);
        if let Some(line) = target.line {
            let doc = &mut editor.documents[index];
            let text = doc.text();
            let line = line.min(text.line_count().saturating_sub(1));
            let pos = text.line_col_to_pos(line, target.col.unwrap_or(0));
            doc.selection = winc_core::Selection::point(pos);
        }
        first_file.get_or_insert(index);
    }

    // 命令行只给了文件：以第一个文件所在的目录作为项目，并在文件树中展开到该文件。
    if !args.one_off()
        && editor.workspace.is_none()
        && let Some(file) = first_file.and_then(|i| editor.documents[i].path.clone())
        && let Some(parent) = file.parent().map(Path::to_path_buf)
        && editor.open_folder(&parent).is_ok()
        && let Some(workspace) = &mut editor.workspace
    {
        workspace.reveal(&file);
    }
    if let Some(index) = first_file {
        editor.active = Some(index);
    }
    problems
}

/// 是否已将要打开的目录和文件交给正在运行的 winc，由该进程切到打开该目录的窗口、新建窗口或
/// 切到前台。
fn hand_over(args: &cli::Args) -> bool {
    let targets: Vec<cli::Target> = args
        .targets
        .iter()
        .map(|t| cli::Target {
            path: instance::absolute(&t.path),
            ..t.clone()
        })
        .collect();
    instance::forward_to_app(&targets, instance::launch_token().as_deref())
}

/// 设置界面语言，并让汉字按界面语言的字形显示，返回字形所用的语言是否发生变化。
///
/// 界面为中文时使用简体中文字形，界面文字与字形一致；界面为英文时界面上没有汉字，文件中的
/// 汉字按系统语言选择字形（繁体中文、日文、韩文的地区各用本地字形）。返回值为 `true` 时
/// 已有的渲染器需要重新加载字体。
pub(crate) fn set_locale(locale: winc_i18n::Locale) -> bool {
    winc_i18n::set_locale(locale);
    winc_render::set_cjk_language(match locale {
        winc_i18n::Locale::ZhCn => Some("zh-cn"),
        winc_i18n::Locale::EnUs => None,
    })
}

/// 按系统语言设置推测编码时参考的地区。
///
/// macOS 上向系统查询首选语言（从图形界面启动的应用没有 `LANG`），其他系统读取环境变量。
fn set_encoding_region() {
    #[cfg(target_os = "macos")]
    let language = crate::macos::preferred_language();
    #[cfg(not(target_os = "macos"))]
    let language = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()));
    if let Some(language) = language {
        winc_editor::encoding::set_region_from_language(&language);
    }
}

/// 打开上次退出时的项目目录，返回是否打开成功。
///
/// 目录已不存在时不做任何事，开始页的最近列表仍保留该条目。标签由 `app::restore_tabs`
/// 按现场恢复。
pub(crate) fn open_session_folder(editor: &mut Editor, session: &Session) -> bool {
    session.folder.is_dir() && editor.open_folder(&session.folder).is_ok()
}
