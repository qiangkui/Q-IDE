//! AI 面板发来的「写入文件」请求：确认路径、写盘，并同步编辑器与资源管理器。

use super::dialogs::*;
use super::*;

impl CodeWindow {
    /// 当前打开文件的路径与实时内容，作为 AI 请求的上下文。
    pub(crate) fn ai_file_context(&self) -> Option<(String, String)> {
        let index = self.active_file?;
        let file = self.open_files.get(index)?;
        let content = self.contents.get(index)?;
        Some((file.id.to_string(), content.to_string()))
    }

    /// 「写入文件」：先让用户确认目标路径，再写盘。
    pub(crate) fn apply_ai_code(&mut self, code: &str, window: &mut Window, cx: &mut Context<Self>) {
        let hint = self.ai_target_hint();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("文件路径，例如 src/main.vb")
                .default_value(hint)
        });
        let handle = cx.entity().downgrade();
        let code = code.to_string();
        let apply: DialogCallback = {
            let field = input.clone();
            Rc::new(move |window, cx| {
                let target = field.read(cx).value().to_string();
                let _ = handle
                    .update(cx, |this, cx| this.write_ai_code(&target, &code, window, cx));
                window.close_dialog(cx);
            })
        };

        show_input_dialog(
            window,
            cx,
            InputDialogSpec {
                id_prefix: "ai-write",
                title: "写入文件",
                description: Some(
                    "会用这段代码覆盖目标文件的全部内容；相对路径按项目根目录解析，\
                     文件不存在会自动新建。",
                ),
                confirm: "写入",
            },
            input,
            apply,
        );
    }

    /// 写入对话框里的默认路径：当前文件（相对项目根目录），没有就新建一个。
    fn ai_target_hint(&self) -> String {
        if let Some(index) = self.active_file
            && let Some(file) = self.open_files.get(index)
        {
            let id = file.id.to_string();
            let path = Path::new(&id);
            if let Some(root) = self
                .explorer
                .roots()
                .iter()
                .find(|root| path.starts_with(root.path()))
                && let Ok(relative) = path.strip_prefix(root.path())
            {
                return relative.to_string_lossy().into_owned();
            }
            return id;
        }
        "ai-output.vb".to_string()
    }

    /// 把对话框里填的路径变成绝对路径：相对项目根目录，没根目录时用当前文件所在目录。
    fn resolve_ai_path(&self, target: &str) -> PathBuf {
        let path = PathBuf::from(target);
        if path.is_absolute() {
            return path;
        }
        if let Some(root) = self.explorer.roots().first() {
            return root.path().join(path);
        }
        if let Some(index) = self.active_file
            && let Some(file) = self.open_files.get(index)
        {
            let id = file.id.to_string();
            if let Some(dir) = Path::new(&id).parent() {
                return dir.join(path);
            }
        }
        path
    }

    /// 写盘：文件已打开就换掉编辑器内容并按原编码保存，否则写完再打开它。
    fn write_ai_code(
        &mut self,
        target: &str,
        code: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = target.trim();
        if target.is_empty() {
            self.output().warning(cx, "没有填写文件路径，已取消写入");
            return;
        }
        let path = self.resolve_ai_path(target);
        if let Err(error) = self.write_ai_file(&path, code, window, cx) {
            self.output()
                .error(cx, format!("写入 {} 失败：{error}", path.display()));
        }
    }

    /// 把内容写进文件（AI 的「写入」按钮与工具调用共用）。
    ///
    /// 已打开的标签会直接换掉编辑器内容并按原编码保存；没打开的写到磁盘再打开。
    pub(crate) fn write_ai_file(
        &mut self,
        path: &Path,
        content: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let id = path.to_string_lossy().into_owned();

        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == id.as_str())
        {
            if let Some(editor) = self.editors.get(index).cloned() {
                editor.update(cx, |editor, cx| {
                    editor.set_value(content.to_string(), window, cx)
                });
            }
            if let Some(slot) = self.contents.get_mut(index) {
                *slot = content.to_string().into();
            }
            self.write_content(index, path)
                .map_err(|error| error.to_string())?;
            if self.active_file == Some(index) {
                self.reset_editor_caches();
                self.refresh_outline(cx);
                self.refresh_diagnostics(cx);
                self.refresh_breakpoint_marks(cx);
                self.refresh_editor_language(cx);
            }
            self.sync_editor_panel(cx);
            self.output().success(cx, format!("已写入 {id}"));
            cx.notify();
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建目录 {}：{error}", parent.display()))?;
        }
        std::fs::write(path, encoding::encode(content, Default::default()))
            .map_err(|error| error.to_string())?;
        self.output().success(cx, format!("已写入 {id}"));
        self.rebuild_tree(cx);
        self.open_file(&id, window, cx);
        Ok(())
    }

    /// 项目根目录（资源管理器里的第一个根文件夹）。
    pub(crate) fn ai_project_root(&self) -> Option<PathBuf> {
        self.explorer
            .roots()
            .first()
            .map(|root| root.path().to_path_buf())
    }

    /// 文件当前的内容：已打开的标签用编辑器里的版本（可能还没保存），否则读磁盘。
    pub(crate) fn file_content(&self, path: &Path) -> Option<String> {
        let id = path.to_string_lossy();
        if let Some(index) = self
            .open_files
            .iter()
            .position(|file| file.id.as_ref() == id.as_ref())
        {
            return self.contents.get(index).map(|content| content.to_string());
        }
        let bytes = std::fs::read(path).ok()?;
        Some(encoding::decode(&bytes).text)
    }

    /// 输出面板最近的内容（脚本输出与报错都在里面）。
    pub(crate) fn ai_output_tail(&self, cx: &App) -> String {
        self.output.read(cx).text()
    }
}
