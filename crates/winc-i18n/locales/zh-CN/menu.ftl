## 菜单栏与程序坞菜单

menu-settings = 设置…
menu-check-updates = 检查更新
menu-window = 窗口
menu-window-minimize = 最小化
menu-window-zoom = 缩放
menu-window-bring-all-to-front = 前置全部窗口

## 状态栏菜单

menu-reopen-with-encoding = 通过编码重新打开
menu-save-with-encoding = 通过编码保存
menu-indent-size = 缩进大小
menu-tab-size = 制表符长度

## 编辑区与文件菜单

menu-table = 表格
menu-open-recent-in-new-window = 在新窗口中打开「{ $name }」
menu-copied-path = 已复制路径
menu-copied-relative-path = 已复制相对路径
menu-copied-reference = 已复制 { $reference }
menu-copy-reference-unsaved = 文件尚未保存，没有路径

## 格式化工具菜单

menu-tool-auto-format-desc = 优先使用项目配置的工具，其次使用已安装的工具
menu-tool-auto-fix-desc = 仅使用项目配置的修复工具
menu-tool-not-installed = { $name }（未安装）

## 日志中的提交菜单

menu-log-checkout-branch = 签出「{ $branch }」
menu-log-new-worktree-branch = 在新工作树中签出「{ $branch }」
menu-log-cherry-pick-onto = 拣选到「{ $branch }」
menu-log-reset-branch = 将「{ $branch }」重置到此提交
menu-log-reset-soft-desc = 更改全部保留在暂存区
menu-log-reset-mixed-desc = 更改保留在工作区，不暂存
menu-log-reset-hard-desc = 丢弃所有未提交的更改
menu-form-base-current-branch = { $branch }（当前分支）
menu-form-base-current-commit = 当前提交

## 分支菜单

menu-git-current-branch = 当前分支
menu-git-switch-to-worktree = 切换到工作树「{ $name }」
menu-git-checkout-stale-worktree = 所在工作树的目录已不存在，请先清理失效的工作树
menu-git-finish-operation = 完成{ $operation }…
menu-git-continue-operation = 继续{ $operation }
menu-git-abort-operation = 中止{ $operation }
menu-git-continue-desc = 请先在文件中解决冲突，再标记为已解决
menu-git-abort-desc = 恢复到操作开始前的状态
menu-git-push-ahead = { $count } 个提交 → { $upstream }
menu-git-push-up-to-date = 到 { $upstream }，没有待推送的提交
menu-git-push-set-upstream = 推送「{ $branch }」并设为上游
menu-git-pull-behind = { $count } 个提交 ← { $upstream }
menu-git-pull-from = 从 { $upstream }
menu-git-pull-no-upstream = 当前分支没有上游
menu-git-fetch-desc = 下载远程更新，不改动本地分支

## 右键菜单项

menu-new-file = 新建文件…
menu-new-folder = 新建目录…
menu-open-in-terminal = 在终端中打开
menu-close-tabs-right = 关闭右侧标签页
menu-close-tabs-left = 关闭左侧标签页
menu-close-saved-tabs = 关闭已保存的标签页
menu-compare-selected = 比较所选文件
menu-reveal-in-tree = 在文件树中显示
menu-tree-collapse-all = 折叠所有目录
menu-tree-locate = 定位当前文件
menu-reveal-source = 在源代码中显示
menu-pdf-highlight = 高亮
menu-pdf-underline = 下划线
menu-pdf-strikeout = 删除线
menu-pdf-comment = 高亮并写批注…
menu-pdf-edit-note = 修改批注…
menu-pdf-remove-note = 删除批注
menu-pdf-install-engine = 下载 PDFium（选中文字、加批注）
menu-add-bookmark = 添加书签
menu-remove-bookmark = 移除书签
menu-detect-language = 按文件名识别
menu-search-dismiss = 从结果中移除
menu-terminal-clear = 清除终端
menu-terminal-maximize = 最大化面板
menu-terminal-close = 关闭终端
menu-terminal-copy-command = 复制命令
menu-terminal-copy-output = 复制输出
menu-terminal-rerun = 重新运行

## 表格菜单

menu-table-insert-row-above = 在上方插入行
menu-table-insert-row-below = 在下方插入行
menu-table-insert-column-left = 在左侧插入列
menu-table-insert-column-right = 在右侧插入列
menu-table-align-left = 左对齐
menu-table-align-center = 居中
menu-table-align-right = 右对齐

## Git 菜单项

menu-new-worktree = 新建工作树…
menu-git-fetch = 获取
menu-git-pull = 拉取
menu-git-push = 推送
menu-git-continue = 继续
menu-git-show-diff = 显示差异
menu-git-open-file = 打开文件
menu-git-pull-rebase = 拉取并变基
menu-log-tag = 标签「{ $name }」
menu-git-roll-back = 回滚
menu-git-roll-back-change = 回滚此处更改
menu-git-file-history = 显示文件历史
menu-git-blame = 逐行追溯
menu-git-stash = 储藏更改…
menu-git-local-branches = 本地分支
menu-git-remote-branches = 远程分支
menu-log-checkout = 签出提交
menu-log-new-worktree = 在新工作树中签出此提交
menu-log-cherry-pick = 拣选到当前分支
menu-log-revert = 还原提交
menu-log-reset-soft = 软重置
menu-log-reset-mixed = 混合重置
menu-log-reset-hard = 硬重置…
menu-log-new-tag = 新建标签…
menu-log-interactive-rebase = 从此处交互式变基…
menu-log-copy-oid = 复制提交号
menu-log-copy-short-oid = 复制短提交号
menu-log-copy-message = 复制提交说明
menu-log-all-branches = 全部分支
menu-log-all-authors = 全部作者
menu-commit-check = 勾选
menu-commit-uncheck = 取消勾选
menu-commit-tree = 以树形显示
menu-git-show-output = 显示 Git 输出
menu-commit-expand-all = 全部展开
menu-commit-collapse-all = 全部折叠
menu-commit-roll-back = 回滚勾选的文件
menu-commit-undo = 撤销提交
menu-commit-and-push = 提交并推送
menu-commit-amend = 修改上次提交
