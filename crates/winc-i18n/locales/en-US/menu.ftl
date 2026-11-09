## Menu bar and Dock menu

menu-settings = Settings…
menu-check-updates = Check for Updates
menu-window = Window
menu-window-minimize = Minimize
menu-window-zoom = Zoom
menu-window-bring-all-to-front = Bring All to Front

## Status bar menus

menu-reopen-with-encoding = Reopen with Encoding
menu-save-with-encoding = Save with Encoding
menu-indent-size = Indent Size
menu-tab-size = Tab Size

## Editor and file menus

menu-table = Table
menu-open-recent-in-new-window = Open “{ $name }” in New Window
menu-copied-path = Copied path
menu-copied-relative-path = Copied relative path
menu-copied-reference = Copied { $reference }
menu-copy-reference-unsaved = The file hasn’t been saved and has no path

## Formatter menu

menu-tool-auto-format-desc = Uses the project’s configured tool first, then an installed one
menu-tool-auto-fix-desc = Uses only the project’s configured fix tool
menu-tool-not-installed = { $name } (Not Installed)

## Commit menu in the log

menu-log-checkout-branch = Check Out “{ $branch }”
menu-log-new-worktree-branch = Check Out “{ $branch }” in New Worktree
menu-log-cherry-pick-onto = Cherry-Pick onto “{ $branch }”
menu-log-reset-branch = Reset “{ $branch }” to This Commit
menu-log-reset-soft-desc = Keep all changes staged
menu-log-reset-mixed-desc = Keep changes in the working tree, unstaged
menu-log-reset-hard-desc = Discard all uncommitted changes
menu-form-base-current-branch = { $branch } (Current Branch)
menu-form-base-current-commit = Current Commit

## Branch menu

menu-git-current-branch = Current Branch
menu-git-switch-to-worktree = Switch to Worktree “{ $name }”
menu-git-checkout-stale-worktree = The worktree’s folder no longer exists. Prune stale worktrees first
menu-git-finish-operation = Finish { $operation }…
menu-git-continue-operation = Continue { $operation }
menu-git-abort-operation = Abort { $operation }
menu-git-continue-desc = Resolve the conflicts in the files, then mark them as resolved
menu-git-abort-desc = Restore the state from before the operation started
menu-git-push-ahead = { $count ->
        [one] { $count } commit → { $upstream }
       *[other] { $count } commits → { $upstream }
    }
menu-git-push-up-to-date = To { $upstream }, nothing to push
menu-git-push-set-upstream = Push “{ $branch }” and set it as upstream
menu-git-pull-behind = { $count ->
        [one] { $count } commit ← { $upstream }
       *[other] { $count } commits ← { $upstream }
    }
menu-git-pull-from = From { $upstream }
menu-git-pull-no-upstream = The current branch has no upstream
menu-git-fetch-desc = Download remote updates without changing local branches

## Context menu items

menu-new-file = New File…
menu-new-folder = New Folder…
menu-open-in-terminal = Open in Terminal
menu-close-tabs-right = Close Tabs to the Right
menu-close-tabs-left = Close Tabs to the Left
menu-close-saved-tabs = Close Saved Tabs
menu-compare-selected = Compare Selected Files
menu-reveal-in-tree = Reveal in File Tree
menu-tree-collapse-all = Collapse All Folders
menu-tree-locate = Reveal Active File
menu-reveal-source = Show in Source
menu-pdf-highlight = Highlight
menu-pdf-underline = Underline
menu-pdf-strikeout = Strikethrough
menu-pdf-comment = Highlight with Comment…
menu-pdf-edit-note = Edit Comment…
menu-pdf-remove-note = Delete Annotation
menu-add-bookmark = Add Bookmark
menu-remove-bookmark = Remove Bookmark
menu-detect-language = Detect from File Name
menu-search-dismiss = Remove from Results
menu-terminal-clear = Clear Terminal
menu-terminal-maximize = Maximize Panel
menu-terminal-close = Close Terminal
menu-terminal-copy-command = Copy Command
menu-terminal-copy-output = Copy Output
menu-terminal-rerun = Run Again

## Table menu

menu-table-insert-row-above = Insert Row Above
menu-table-insert-row-below = Insert Row Below
menu-table-insert-column-left = Insert Column Left
menu-table-insert-column-right = Insert Column Right
menu-table-align-left = Align Left
menu-table-align-center = Align Center
menu-table-align-right = Align Right

## Git menu items

menu-new-worktree = New Worktree…
menu-git-fetch = Fetch
menu-git-pull = Pull
menu-git-push = Push
menu-git-continue = Continue
menu-git-show-diff = Show Diff
menu-git-open-file = Open File
menu-git-pull-rebase = Pull with Rebase
menu-log-tag = Tag “{ $name }”
menu-git-roll-back = Roll Back
menu-git-roll-back-change = Roll Back This Change
menu-git-file-history = Show File History
menu-git-blame = Blame
menu-git-stash = Stash Changes…
menu-git-local-branches = Local Branches
menu-git-remote-branches = Remote Branches
menu-log-checkout = Check Out Commit
menu-log-new-worktree = Check Out This Commit in New Worktree
menu-log-cherry-pick = Cherry-Pick onto Current Branch
menu-log-revert = Revert Commit
menu-log-reset-soft = Soft Reset
menu-log-reset-mixed = Mixed Reset
menu-log-reset-hard = Hard Reset…
menu-log-new-tag = New Tag…
menu-log-interactive-rebase = Interactively Rebase from Here…
menu-log-copy-oid = Copy Commit Hash
menu-log-copy-short-oid = Copy Short Commit Hash
menu-log-copy-message = Copy Commit Message
menu-log-all-branches = All Branches
menu-log-all-authors = All Authors
menu-commit-check = Check
menu-commit-uncheck = Uncheck
menu-commit-tree = Show as Tree
menu-git-show-output = Show Git Output
menu-commit-expand-all = Expand All
menu-commit-collapse-all = Collapse All
menu-commit-roll-back = Roll Back Checked Files
menu-commit-undo = Undo Commit
menu-commit-and-push = Commit and Push
menu-commit-amend = Amend Last Commit
