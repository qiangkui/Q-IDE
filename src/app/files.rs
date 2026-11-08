//! 文件、标签与资源管理器操作。

use super::*;
use super::dialogs::*;

impl CodeWindow {
    /// 切换标签：保留各标签自己的编辑器状态，只切换当前使用的编辑器实体。
    pub(crate) fn activate_file(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.open_files.len() || self.active_file == Some(index) {
            return;
        }
        let Some(editor) = self.editors.get(index).cloned() else {
            return;
        };

        self.save_active_content(cx);
        self.active_file = Some(index);
        self.editor = editor;
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));

        // 这些缓存属于「当前标签」，切换后要按新标签重新计算。
        self.reset_editor_caches();

        self.refresh_outline(cx);
        self.refresh_editor_language(cx);
        self.refresh_diagnostics(cx);
        self.refresh_breakpoint_marks(cx);
        self.refresh_bracket_match(cx);
        self.sync_editor_panel(cx);
        cx.notify();
    }

    /// 从资源管理器打开文件；已打开则直接聚焦对应标签。
    /// 磁盘上的路径按绝对路径读取内容。
    pub(crate) fn open_file(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let path = Path::new(id);
        // 空文件夹没有子项，树会把它当成普通条目，点上去时按文件夹处理。
        if path.is_dir() {
            self.toggle_folder(id, cx);
            return;
        }

        if let Some(parent) = path.parent()
            && parent.is_dir()
        {
            self.new_file_target = Some(parent.to_path_buf());
        }

        if let Some(index) = self.open_files.iter().position(|open| open.id == id) {
            self.activate_file(index, window, cx);
            return;
        }

        let (name, language, decoded) = match std::fs::read(path) {
            Ok(bytes) => (
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| id.to_string()),
                workspace::language_for_path(path).to_string(),
                encoding::decode(&bytes),
            ),
            Err(error) => {
                let output = self.output();
                output.error(cx, format!("无法打开 {id}：{error}"));
                return;
            }
        };
        let content = decoded.text;
        let stamp = workspace::disk_stamp(path);

        self.save_active_content(cx);
        self.open_files.push(OpenFile {
            id: id.into(),
            name: name.into(),
            language: language.clone().into(),
            encoding: decoded.encoding,
        });
        self.contents.push(content.clone().into());
        self.saved.push(content.clone().into());
        self.disk_stamps.push(stamp);

        let index = self.open_files.len() - 1;
        self.active_file = Some(index);

        // 每个标签持有自己的编辑器实体，切换回来时光标、滚动和撤销历史都还在。
        let editor = self.add_editor(content.into(), language.into(), window, cx);
        self.editor = editor;
        self.editor.update(cx, |editor, cx| editor.focus(window, cx));

        self.reset_editor_caches();

        self.refresh_outline(cx);
        self.refresh_editor_language(cx);
        self.refresh_diagnostics(cx);
        self.refresh_breakpoint_marks(cx);
        self.sync_editor_panel(cx);
        if crate::layout::remember_recent(&mut self.recent_files, id) {
            self.save_layout(cx);
        }
        cx.notify();
    }

    /// 新建 VB 文件。
    ///
    /// 资源管理器里有文件夹时，建在最近点过的文件夹里（点过文件就用它所在的目录，
    /// 都没点过就用第一个根文件夹）；资源管理器为空时先弹出文件夹选择框，
    /// 选好后把该文件夹加入资源管理器再新建。
    pub(crate) fn new_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dir) = self.target_dir() {
            self.create_file_in(dir, window, cx);
            return;
        }

        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择新建文件所在的文件夹".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(dir) = paths.into_iter().next()
            {
                this.update_in(cx, |this, window, cx| {
                    if this.explorer.add_folder(dir.clone()) {
                        this.rebuild_tree(cx);
                        this.save_layout(cx);
                    }
                    this.create_file_in(dir, window, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// 新建文件的目标目录。
    pub(super) fn target_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = &self.new_file_target
            && dir.is_dir()
            && self
                .explorer
                .roots()
                .iter()
                .any(|root| dir.starts_with(root.path()))
        {
            return Some(dir.clone());
        }
        self.explorer
            .roots()
            .first()
            .map(|root| root.path().to_path_buf())
    }

    /// 在 `dir` 下新建一个空的 `新建文件*.vb`，刷新树并打开它。
    pub(crate) fn create_file_in(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = free_vb_path(&dir);
        if let Err(error) = std::fs::write(&path, "") {
            let output = self.output();
            output.error(cx, format!("无法新建文件：{error}"));
            return;
        }
        self.explorer.expand(&dir);
        self.rebuild_tree(cx);
        self.save_layout(cx);
        self.open_file(&path.to_string_lossy(), window, cx);
    }

    /// 在目标目录下新建一个空文件夹（名字自动避开重名）。
    pub(crate) fn new_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.target_dir() else {
            self.output()
                .warning(cx, "先在资源管理器里添加一个文件夹");
            return;
        };
        self.create_folder_in(dir, cx);
    }

    /// 在 `dir` 下新建一个空文件夹。
    pub(crate) fn create_folder_in(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let mut index = 1;
        let path = loop {
            let name = if index == 1 {
                "新建文件夹".to_string()
            } else {
                format!("新建文件夹 ({index})")
            };
            let candidate = dir.join(name);
            if !candidate.exists() {
                break candidate;
            }
            index += 1;
        };
        match std::fs::create_dir(&path) {
            Ok(()) => {
                self.explorer.expand(&dir);
                self.rebuild_tree(cx);
                self.save_layout(cx);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.output().success(cx, format!("已新建文件夹 {name}"));
            }
            Err(error) => {
                self.output()
                    .error(cx, format!("无法新建文件夹：{error}"));
            }
        }
    }

    /// 重新读盘刷新资源管理器（外部新增/删除的文件立刻可见）。
    pub(crate) fn refresh_explorer(&mut self, cx: &mut Context<Self>) {
        self.rebuild_tree(cx);
        self.output().info(cx, "资源管理器已刷新");
    }

    /// 把一段文字（比如文件路径）复制到剪贴板。
    pub(crate) fn copy_path_text(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        self.output().command(cx, format!("> 已复制路径：{text}"));
    }

    /// 打开「重命名」对话框（标签菜单/文件菜单用，作用于当前标签）。
    pub(crate) fn rename_file_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.open_files.get(index).map(|file| file.id.to_string()) else {
            return;
        };
        self.rename_path_dialog(path, window, cx);
    }

    /// 打开「重命名」对话框：文件或文件夹都支持，改的是磁盘上的名字。
    pub(crate) fn rename_path_dialog(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(name.clone())
                .placeholder("新名字")
        });
        let handle = cx.entity().downgrade();
        let field = input.clone();
        let apply: DialogCallback = Rc::new(move |window, cx| {
            let value = field.read(cx).value().to_string();
            let path = path.clone();
            let _ = handle.update(cx, |this, cx| this.rename_path(&path, &value, window, cx));
        });
        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "rename-path",
                title: "重命名",
                description: Some("只改名字；有未保存的修改会先写回原文件。"),
                confirm: "重命名",
            },
            input,
            apply,
        );
    }

    pub(super) fn rename_path(
        &mut self,
        path: &str,
        new_name: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let old_path = PathBuf::from(path);
        let name = new_name.trim().to_string();
        if name.is_empty() {
            self.output().warning(cx, "名字不能为空");
            return;
        }
        if name.contains(['/', '\\']) || Path::new(&name).file_name().is_none() {
            self.output().warning(cx, "名字里不能带路径分隔符");
            return;
        }
        let Some(parent) = old_path.parent() else {
            return;
        };
        let new_path = parent.join(&name);
        if new_path == old_path {
            return;
        }
        if new_path.exists() {
            self.output().error(cx, format!("{name} 已经存在"));
            return;
        }
        // 打开着的被改文件如果有未保存内容，先写回旧路径，避免重命名后丢改动。
        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == path)
            && self.file_is_dirty(index)
        {
            let _ = self.write_content(index, &old_path);
        }
        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                let new_id = new_path.to_string_lossy().into_owned();
                self.rewrite_path_prefix(path, &new_id);
                self.rebuild_tree(cx);
                self.refresh_editor_language(cx);
                self.sync_editor_panel(cx);
                self.save_layout(cx);
                self.output().success(cx, format!("已重命名为 {name}"));
            }
            Err(error) => {
                self.output().error(cx, format!("重命名失败：{error}"));
            }
        }
    }

    /// 路径改名后同步所有记录：打开的文件、断点、条件断点、最近打开。
    ///
    /// 文件夹改名时按前缀替换，目录下已打开的文件也跟着走。
    pub(super) fn rewrite_path_prefix(&mut self, old: &str, new: &str) {
        for file in &mut self.open_files {
            if let Some(updated) = reprefix(&file.id, old, new) {
                file.name = Path::new(&updated)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| updated.clone())
                    .into();
                file.language = workspace::language_for_path(Path::new(&updated)).into();
                file.id = updated.into();
            }
        }
        self.breakpoints = std::mem::take(&mut self.breakpoints)
            .into_iter()
            .map(|(path, lines)| (reprefix(&path, old, new).unwrap_or(path), lines))
            .collect();
        self.breakpoint_conditions = std::mem::take(&mut self.breakpoint_conditions)
            .into_iter()
            .map(|(path, conditions)| (reprefix(&path, old, new).unwrap_or(path), conditions))
            .collect();
        for item in &mut self.recent_files {
            if let Some(updated) = reprefix(item, old, new) {
                *item = updated;
            }
        }
    }

    /// 打开「删除」确认框（标签菜单/文件菜单用，作用于当前标签）。
    pub(crate) fn delete_file_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.open_files.get(index).map(|file| file.id.to_string()) else {
            return;
        };
        self.delete_path_dialog(path, false, window, cx);
    }

    /// 打开「删除」确认框：文件或文件夹（文件夹连同内容一起删）。
    pub(crate) fn delete_path_dialog(
        &mut self,
        path: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let description = if is_dir {
            format!("确定要删除文件夹 {name} 及其中的全部内容吗？删除后无法撤销。")
        } else {
            format!("确定要删除 {name} 吗？删除后无法撤销。")
        };
        let handle = cx.entity().downgrade();
        let on_confirm: DialogCallback = Rc::new(move |window, cx| {
            let path = path.clone();
            let _ = handle.update(cx, |this, cx| this.delete_path(&path, is_dir, window, cx));
        });
        show_confirm_dialog(window, cx, "删除", description, "删除", on_confirm);
    }

    pub(super) fn delete_path(&mut self, path: &str, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        let result = if is_dir {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        if let Err(error) = result {
            self.output().error(cx, format!("删除失败：{error}"));
            return;
        }
        self.forget_paths(&[path.to_string()], window, cx);
        self.output().success(cx, format!("已删除 {path}"));
    }

    /// 把资源管理器里拖动的条目移动到目标目录。
    pub(crate) fn move_path(&mut self, source: &str, target_dir: &str, cx: &mut Context<Self>) {
        let source_path = Path::new(source);
        let Some(name) = source_path.file_name() else {
            return;
        };
        let target = Path::new(target_dir).join(name);
        if target == source_path {
            return;
        }
        if target.exists() {
            self.output().error(
                cx,
                format!("目标里已经有 {} 了", name.to_string_lossy()),
            );
            return;
        }
        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == source)
            && self.file_is_dirty(index)
        {
            let _ = self.write_content(index, source_path);
        }
        match std::fs::rename(source_path, &target) {
            Ok(()) => {
                let new_path = target.to_string_lossy().into_owned();
                self.rewrite_path_prefix(source, &new_path);
                self.explorer.expand(Path::new(target_dir));
                self.rebuild_tree(cx);
                self.refresh_editor_language(cx);
                self.sync_editor_panel(cx);
                self.save_layout(cx);
                self.output().success(
                    cx,
                    format!(
                        "已移动 {} 到 {}",
                        name.to_string_lossy(),
                        target_dir
                    ),
                );
            }
            Err(error) => {
                self.output().error(cx, format!("移动失败：{error}"));
            }
        }
    }

    /// 这些路径被删除后：关掉相关标签，清掉断点与最近打开记录。
    pub(super) fn forget_paths(&mut self, paths: &[String], window: &mut Window, cx: &mut Context<Self>) {
        let affected = |file: &str| {
            paths
                .iter()
                .any(|path| workspace::reprefix_path(file, path, "").is_some())
        };
        let mut indexes: Vec<usize> = self
            .open_files
            .iter()
            .enumerate()
            .filter(|(_, file)| affected(&file.id))
            .map(|(index, _)| index)
            .collect();
        while let Some(index) = indexes.pop() {
            self.close_file_now(index, window, cx);
        }
        self.breakpoints.retain(|file, _| !affected(file));
        self.breakpoint_conditions
            .retain(|file, _| !affected(file));
        self.recent_files.retain(|item| !affected(item));
        self.rebuild_tree(cx);
        self.save_layout(cx);
    }

    /// 关闭除 `keep` 之外的其它标签。
    pub(crate) fn close_other_files(
        &mut self,
        keep: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(keep_id) = self.open_files.get(keep).map(|file| file.id.to_string()) else {
            return;
        };
        let mut index = 0;
        while index < self.open_files.len() {
            if self.open_files[index].id.as_ref() == keep_id {
                index += 1;
                continue;
            }
            let before = self.open_files.len();
            self.close_file(index, window, cx);
            // 有未保存的修改时会弹确认框，此时先停下，等用户决定。
            if self.open_files.len() == before {
                break;
            }
        }
    }

    /// 关闭 `index` 右侧的所有标签。
    pub(crate) fn close_files_to_right(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while self.open_files.len() > index + 1 {
            let last = self.open_files.len() - 1;
            let before = self.open_files.len();
            self.close_file(last, window, cx);
            if self.open_files.len() == before {
                break;
            }
        }
    }

    /// 把某个标签对应文件的完整路径复制到剪贴板。
    pub(crate) fn copy_file_path(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(path) = self
            .open_files
            .get(index)
            .map(|file| file.id.to_string())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        self.output().command(cx, format!("> 已复制路径：{path}"));
    }

    /// 打开文件对话框（与资源管理器「+」添加文件夹同一套系统选择框）。
    ///
    /// 选中的文件会逐个打开；它们所在的文件夹同时挂到资源管理器里。
    pub(crate) fn open_files_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("打开文件".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && !paths.is_empty()
            {
                this.update_in(cx, |this, window, cx| {
                    for path in paths {
                        if let Some(parent) = path.parent()
                            && parent.is_dir()
                        {
                            this.explorer.add_folder(parent.to_path_buf());
                            this.explorer.expand(parent);
                        }
                        this.open_file(&path.to_string_lossy(), window, cx);
                    }
                    this.rebuild_tree(cx);
                    this.save_layout(cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// 把所有有改动的文件写回磁盘，返回保存成功的数量。
    pub(super) fn save_dirty_files(&mut self, cx: &mut Context<Self>) -> usize {
        let mut saved = 0;
        let mut failures = Vec::new();
        for index in 0..self.open_files.len() {
            if !self.file_is_dirty(index) {
                continue;
            }
            let id = self.open_files[index].id.to_string();
            let name = self.open_files[index].name.to_string();
            match self.write_content(index, Path::new(&id)) {
                Ok(()) => saved += 1,
                Err(error) => failures.push(format!("{name}：{error}")),
            }
        }
        if saved > 0 {
            self.sync_editor_panel(cx);
            self.output().success(cx, format!("已自动保存 {saved} 个文件"));
        }
        for failure in failures {
            self.output().error(cx, format!("自动保存失败：{failure}"));
        }
        saved
    }

    /// 每秒检查一次自动保存；设置里没开就只是更新时间戳。
    pub(super) fn spawn_auto_save_timer(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.auto_save_tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn auto_save_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 每秒读一次窗口矩形：拖动窗口（不只是缩放）也能被记下来。
        self.remember_window_bounds(window, cx);
        // 窗口被拖动或缩放过：顺手把布局写盘（每秒最多一次，开销可以忽略）。
        if self.window_bounds_dirty {
            self.window_bounds_dirty = false;
            self.save_layout(cx);
        }
        let dirty = (0..self.open_files.len())
            .filter(|index| self.file_is_dirty(*index))
            .count();
        // 每秒顺带看一眼磁盘：外部改过的文件重新加载（或问用户）。
        self.check_external_changes(window, cx);
        let settings = self.settings.read(cx).editor.clone();
        if !settings.auto_save_due(self.last_auto_save.elapsed(), dirty) {
            return;
        }
        self.last_auto_save = Instant::now();
        self.save_dirty_files(cx);
    }

    /// 保存当前文件（`Ctrl+S`），把编辑框内容写回磁盘。
    pub(crate) fn save_file(&mut self, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let Some(index) = self.active_file else {
            let output = self.output();
            output.warning(cx, "没有打开的文件，无法保存");
            return;
        };
        let id = self.open_files[index].id.to_string();
        let name = self.open_files[index].name.to_string();
        match self.write_content(index, Path::new(&id)) {
            Ok(()) => {
                self.sync_editor_panel(cx);
                let output = self.output();
                output.success(cx, format!("已保存 {name}"));
            }
            Err(error) => {
                let output = self.output();
                output.error(cx, format!("保存 {name} 失败：{error}"));
            }
        }
    }

    /// 把第 `index` 个标签的内容按原编码写到 `path`；成功后同步「已保存」快照。
    pub(super) fn write_content(&mut self, index: usize, path: &Path) -> std::io::Result<()> {
        let content = self.contents.get(index).cloned().unwrap_or_default();
        let file_encoding = self
            .open_files
            .get(index)
            .map_or(Default::default(), |file| file.encoding);
        std::fs::write(path, encoding::encode(&content, file_encoding))?;
        if let Some(slot) = self.saved.get_mut(index) {
            *slot = content;
        }
        // 自己写盘后的时间戳要记下来，否则会被当成「外部修改」。
        if let Some(slot) = self.disk_stamps.get_mut(index) {
            *slot = workspace::disk_stamp(path);
        }
        Ok(())
    }

    /// 另存为（`Ctrl+Shift+S`）：选一个路径，写入后把标签切到新文件。
    pub(crate) fn save_file_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_content(cx);
        let Some(index) = self.active_file else {
            let output = self.output();
            output.warning(cx, "没有打开的文件，无法另存为");
            return;
        };
        let current = PathBuf::from(self.open_files[index].id.to_string());
        let directory = current
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self.open_files[index].name.to_string();
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                this.update_in(cx, |this, _window, cx| this.save_as_to(index, path, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// 把第 `index` 个标签的内容写到 `path`，并把标签改名。
    pub(super) fn save_as_to(&mut self, index: usize, path: PathBuf, cx: &mut Context<Self>) {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        if let Err(error) = self.write_content(index, &path) {
            let output = self.output();
            output.error(cx, format!("保存 {name} 失败：{error}"));
            return;
        }
        if let Some(file) = self.open_files.get_mut(index) {
            file.id = path.to_string_lossy().into_owned().into();
            file.name = name.clone().into();
            file.language = workspace::language_for_path(&path).to_string().into();
        }
        self.applied_language = None;
        self.outline_source = None;
        self.refresh_editor_language(cx);
        self.refresh_outline(cx);
        self.sync_editor_panel(cx);
        if let Some(parent) = path.parent()
            && parent.is_dir()
        {
            self.explorer.add_folder(parent.to_path_buf());
            self.explorer.expand(parent);
            self.rebuild_tree(cx);
            self.save_layout(cx);
        }
        let output = self.output();
        output.success(cx, format!("已另存为 {name}"));
        cx.notify();
    }

    /// 弹出系统文件夹选择框，把选中的文件夹加入资源管理器。
    pub(crate) fn add_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("添加文件夹".into()),
        });

        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                this.update_in(cx, |this, window, cx| this.add_folders(paths, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// 把选中的文件夹逐个加入资源管理器（已在树里的会跳过）。
    pub(super) fn add_folders(&mut self, paths: Vec<PathBuf>, _window: &mut Window, cx: &mut Context<Self>) {
        let mut changed = false;
        for path in paths {
            changed |= self.explorer.add_folder(path);
        }
        if changed {
            self.rebuild_tree(cx);
            self.save_layout(cx);
        }
    }

    /// 把根项从树里移除（不删磁盘文件）。
    pub(crate) fn remove_root(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.explorer.remove_root(id) {
            self.rebuild_tree(cx);
            self.save_layout(cx);
        }
    }

    /// 展开或折叠文件夹；被点到的文件夹同时记作新建文件的目标目录。
    pub(crate) fn toggle_folder(&mut self, id: &str, cx: &mut Context<Self>) {
        self.new_file_target = Some(PathBuf::from(id));
        self.explorer.toggle(id);
        self.rebuild_tree(cx);
        self.save_layout(cx);
    }

    /// 用模型重新生成整棵树。
    pub(super) fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let items = self.explorer.items();
        self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
        self.sync_explorer_panel(cx);
        cx.notify();
    }

    /// 保存所有有改动的文件（`Ctrl+Alt+S`、文件菜单与标签右键菜单都用它）。
    pub(crate) fn save_all_files(&mut self, cx: &mut Context<Self>) {
        if self.open_files.is_empty() {
            self.output().warning(cx, "没有打开的文件，无法保存");
            return;
        }
        self.save_active_content(cx);
        if self.dirty_count() == 0 {
            let output = self.output();
            output.info(cx, "没有需要保存的文件");
            return;
        }
        self.save_all_dirty(cx);
    }

}
