## Previews: image, SVG and Markdown previews.

## Preview mode

preview-mode-unsupported = Only SVG, Markdown, CSV, and TSV files can be previewed
preview-mode-source = Source
preview-mode-split = Split
preview-mode-preview = Preview

## Reasons an image can’t be opened or decoded

preview-image-format-unsupported = Unsupported image format
preview-image-not-regular-file = Not a regular file
preview-image-too-large = Image is too large
preview-image-decode-failed = Can’t decode image ({ $error })
preview-image-format-unknown = Unrecognized image format
preview-image-empty = Image is empty
preview-image-dimensions-too-large = { $width } × { $height }, dimensions too large
preview-image-size-mismatch = Decoded size doesn’t match the declared size
preview-image-palette-unexpanded = Palette not expanded
preview-image-truncated = File is incomplete
preview-image-size-unreadable = Can’t read image dimensions
preview-image-no-frames = Image contains no frames
preview-image-bitmap-header-unknown = Unrecognized bitmap header
preview-image-width-invalid = Invalid width
preview-image-bmp-compressed = Compressed BMP isn’t supported
preview-image-palette-incomplete = Palette is incomplete
preview-image-bmp-depth-unsupported = { $bits }-bit BMP isn’t supported
preview-image-icon-empty = Icon contains no images

## Reasons an SVG can’t be opened

preview-svg-nesting-too-deep = Elements nested more than { $max } levels deep
preview-svg-entities-too-large = Entities expand to too much content
preview-svg-xml-error = Malformed XML ({ $error })
preview-svg-not-svg = Not an SVG file

## SVG content that couldn’t be drawn (listed in the status bar)

preview-svg-missing-selectors = some CSS selectors
preview-svg-missing-too-many-effects = too many clips, masks, patterns and filters
preview-svg-missing-animation = animations
preview-svg-missing-foreign-object = embedded web content
preview-svg-missing-markers = markers
preview-svg-missing-remote-images = remote images
preview-svg-missing-image-not-found = missing images
preview-svg-missing-nested-svg = embedded SVG images
preview-svg-missing-image-undecodable = undecodable images
preview-svg-missing-images-too-large = too many embedded images
preview-svg-missing-filter-effects = some filter effects
preview-svg-missing-clip-text = text in clip paths
preview-svg-missing-text = text
preview-svg-missing-text-path = text on paths

## Markdown preview

preview-markdown-image-alt = Image
preview-markdown-code-copied = Copied code
preview-markdown-heading-not-found = This heading isn’t in the document
preview-markdown-link-file-not-found = The linked file wasn’t found
preview-markdown-link-open-failed = Couldn’t open the link: { $error }
preview-markdown-cache-folder-invalid = Invalid cache folder
preview-markdown-download-failed = Download failed

## Markdown table toolbar

preview-table-insert-row = Insert Row
preview-table-insert-column = Insert Column
preview-table-delete-row = Delete Row
preview-table-delete-column = Delete Column
preview-table-delete = Delete Table

## Table preview
preview-table-invalid = Not a valid xlsx file
preview-table-unsupported = Encrypted and zip64 xlsx files aren’t supported
preview-office-damaged = The file is damaged ({ $part })
preview-table-too-large = The file is too large
preview-table-no-sheets = The workbook has no sheets to show
preview-table-empty = This sheet is empty
preview-table-truncated = The table is too large; only the first part is shown
preview-table-sheet = Sheet { $number }

## Open in browser
preview-browser-open = Open in Browser
preview-browser-unsaved = Opened in the browser. It shows the last saved version; you have unsaved changes.
preview-browser-failed = Couldn’t open the browser: { $error }

## Word document preview

preview-docx-invalid = Not a valid docx file
preview-docx-unsupported = Encrypted and zip64 docx files aren’t supported
preview-docx-too-large = The file is too large
preview-docx-comment = Comment ({ $author }):
preview-docx-chart = Chart
preview-docx-chart-titled = Chart: { $title }
preview-docx-chart-series = { $count } series: { $names }
preview-docx-chart-category = Category
preview-docx-image-unsupported = The image “{ $name }” is in { $format } format and can’t be previewed

## PDF preview

preview-pdf-unsupported = PDF preview isn’t supported on this system
preview-pdf-invalid = Not a valid PDF file
preview-pdf-encrypted = Encrypted PDFs can’t be previewed
preview-pdf-empty = The PDF has no pages
preview-pdf-render-failed = The page couldn’t be rendered
preview-pdf-annotate-failed = The annotation couldn’t be changed
preview-pdf-save-failed = The annotations couldn’t be saved to the PDF
