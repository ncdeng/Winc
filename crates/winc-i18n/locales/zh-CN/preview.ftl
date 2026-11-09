## 预览：图片、SVG 和 Markdown 的预览。

## 预览模式

preview-mode-unsupported = 仅支持预览 SVG、Markdown、CSV 和 TSV 文件
preview-mode-source = 代码
preview-mode-split = 分栏
preview-mode-preview = 预览

## 图片无法打开或解码的原因

preview-image-format-unsupported = 不支持的图片格式
preview-image-not-regular-file = 不是普通文件
preview-image-too-large = 图片过大
preview-image-decode-failed = 无法解码图片（{ $error }）
preview-image-format-unknown = 无法识别的图片格式
preview-image-empty = 图片为空
preview-image-dimensions-too-large = { $width } × { $height }，尺寸过大
preview-image-size-mismatch = 解码后的尺寸与声明不符
preview-image-palette-unexpanded = 调色板未展开
preview-image-truncated = 文件不完整
preview-image-size-unreadable = 无法读取图片尺寸
preview-image-no-frames = 图片不包含任何帧
preview-image-bitmap-header-unknown = 无法识别的位图头
preview-image-width-invalid = 宽度无效
preview-image-bmp-compressed = 不支持压缩的 BMP
preview-image-palette-incomplete = 调色板不完整
preview-image-bmp-depth-unsupported = 不支持 { $bits } 位的 BMP
preview-image-icon-empty = 图标中不包含图像

## SVG 无法打开的原因

preview-svg-nesting-too-deep = 元素嵌套超过 { $max } 层
preview-svg-entities-too-large = 实体展开后的内容过多
preview-svg-xml-error = XML 格式错误（{ $error }）
preview-svg-not-svg = 不是 SVG 文件

## SVG 中未能绘制的内容（在状态栏列出）

preview-svg-missing-selectors = 部分 CSS 选择器
preview-svg-missing-too-many-effects = 过多的裁剪、蒙版、图案和滤镜
preview-svg-missing-animation = 动画
preview-svg-missing-foreign-object = 嵌入的网页内容
preview-svg-missing-markers = 箭头标记
preview-svg-missing-remote-images = 网络图片
preview-svg-missing-image-not-found = 无法找到的图片
preview-svg-missing-nested-svg = 嵌入的 SVG 图片
preview-svg-missing-image-undecodable = 无法解码的图片
preview-svg-missing-images-too-large = 过多的内嵌图片
preview-svg-missing-filter-effects = 部分滤镜效果
preview-svg-missing-clip-text = 裁剪路径中的文字
preview-svg-missing-text = 文字
preview-svg-missing-text-path = 沿路径排列的文字

## Markdown 预览

preview-markdown-image-alt = 图片
preview-markdown-code-copied = 已复制代码
preview-markdown-heading-not-found = 文档中没有此标题
preview-markdown-link-file-not-found = 未找到链接指向的文件
preview-markdown-link-open-failed = 无法打开链接：{ $error }
preview-markdown-cache-folder-invalid = 缓存目录无效
preview-markdown-download-failed = 下载失败

## Markdown 表格工具栏

preview-table-insert-row = 插入行
preview-table-insert-column = 插入列
preview-table-delete-row = 删除行
preview-table-delete-column = 删除列
preview-table-delete = 删除表格

## 表格预览
preview-table-invalid = 不是有效的 xlsx 文件
preview-table-unsupported = 不支持加密或 zip64 格式的 xlsx 文件
preview-office-damaged = 文件已损坏（{ $part }）
preview-table-too-large = 文件过大
preview-table-no-sheets = 工作簿中没有可显示的工作表
preview-table-empty = 此工作表为空
preview-table-truncated = 表格过大，只显示了前一部分
preview-table-sheet = 工作表 { $number }

## 在浏览器中打开
preview-browser-open = 在浏览器中打开
preview-browser-unsaved = 已在浏览器中打开；有未保存的修改，浏览器中显示的是上次保存的内容
preview-browser-failed = 无法打开浏览器：{ $error }

## Word 文档预览

preview-docx-invalid = 不是有效的 docx 文件
preview-docx-unsupported = 不支持加密或 zip64 格式的 docx 文件
preview-docx-too-large = 文件过大
preview-docx-comment = 批注（{ $author }）：
preview-docx-chart = 图表
preview-docx-chart-titled = 图表：{ $title }
preview-docx-chart-series = { $count } 组数据：{ $names }
preview-docx-chart-category = 类别
preview-docx-image-unsupported = 图片「{ $name }」为 { $format } 格式，无法预览

## PDF 预览

preview-pdf-unsupported = 当前系统不支持预览 PDF
preview-pdf-invalid = 不是有效的 PDF 文件
preview-pdf-encrypted = 不支持预览加密的 PDF
preview-pdf-empty = PDF 中没有页面
preview-pdf-render-failed = 无法渲染这一页
preview-pdf-annotate-failed = 无法修改这里的批注
preview-pdf-save-failed = 无法保存 PDF 中的批注
preview-pdf-saved = 批注已保存到 PDF
preview-pdf-comment-title = 批注
preview-pdf-comment-placeholder = 写下这里要怎么改
preview-pdf-comment-hint = 回车保存，Shift+回车换行，Esc 取消
preview-pdf-engine-hint = 要选中文字、加批注，需要先下载 PDFium（约 4 MB）
preview-pdf-engine-install = 下载
preview-pdf-engine-downloading = 正在下载 PDFium…
preview-pdf-engine-installed = PDFium 已安装，现在可以选中文字、加批注
preview-pdf-engine-failed = 无法安装 PDFium：{ $reason }
preview-pdf-engine-download-failed = 下载没有成功，请检查网络后重试
preview-pdf-engine-bad-archive = 下载的文件与预期的版本不符
