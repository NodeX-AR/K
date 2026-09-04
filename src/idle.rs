use eframe::egui;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use std::path::PathBuf;
use crate::{lexer, parser, compiler, vm};

pub fn launch() {
    let icon_bytes = include_bytes!("k-logo.png");
    let icon_data = match image::load_from_memory(icon_bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData { rgba: rgba.into_raw(), width, height }
        }
        Err(_) => egui::IconData { rgba: Vec::new(), width: 0, height: 0 },
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(1040.0, 700.0))
            .with_min_inner_size(egui::vec2(640.0, 420.0))
            .with_icon(icon_data),
        ..Default::default()
    };

    eframe::run_native("K Language IDE", options, Box::new(|_cc| Box::new(KIdeApp::default()))).ok();
}

/// Lex, parse, compile, and run — every stage returns its error as text
/// instead of panicking, so a typo in the editor shows a message, never
/// crashes the IDE.
fn run_code(code: &str) -> String {
    let tokens = match lexer::tokenize(code) {
        Ok(t) => t,
        Err(e) => return format!("Lex error: {}", e),
    };
    let stmts = match parser::parse(tokens) {
        Ok(s) => s,
        Err(e) => return format!("Parse error: {}", e),
    };
    let function = match compiler::Compiler::compile_program(&stmts) {
        Ok(f) => f,
        Err(e) => return format!("Compile error: {}", e),
    };
    vm::VM::new().run_program(function)
}

fn looks_like_error(output: &str) -> bool {
    output.starts_with("Lex error")
        || output.starts_with("Parse error")
        || output.starts_with("Compile error")
        || output.contains("Traceback")
        || output.contains("Runtime error")
}

/// Net change in brace depth contributed by one line, ignoring anything
/// inside a string literal or after a `//` comment marker — so a `{` typed
/// inside a string or a comment doesn't throw off indentation.
fn net_brace_change(line: &str) -> i32 {
    let mut net = 0i32;
    let mut chars = line.chars().peekable();
    let mut in_string: Option<char> = None;
    while let Some(c) = chars.next() {
        if let Some(q) = in_string {
            if c == '\\' { chars.next(); continue; }
            if c == q { in_string = None; }
            continue;
        }
        match c {
            '"' | '\'' => in_string = Some(c),
            '/' if chars.peek() == Some(&'/') => break, // rest of the line is a comment
            '{' => net += 1,
            '}' => net -= 1,
            _ => {}
        }
    }
    net
}

/// Re-indents the whole buffer from scratch, purely from `{`/`}` nesting:
/// each line gets `4 * depth` leading spaces, where a line that itself
/// starts with `}` is dedented one level before the depth for that line
/// is applied. K has no significant whitespace (blocks are `{ }`, not
/// indentation), so this only affects how the source *looks* — it can
/// never change what a script does.
fn reindent(code: &str) -> String {
    let mut depth: i32 = 0;
    let mut out_lines: Vec<String> = Vec::new();
    for raw_line in code.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            out_lines.push(String::new());
            continue;
        }
        let this_depth = if trimmed.starts_with('}') { (depth - 1).max(0) } else { depth };
        out_lines.push(format!("{}{}", "    ".repeat(this_depth as usize), trimmed));
        depth = (depth + net_brace_change(trimmed)).max(0);
    }
    out_lines.join("\n")
}

const KEYWORDS: &[&str] = &[
    "let", "const", "fn", "if", "elif", "else", "while", "for", "in", "return",
    "break", "continue", "class", "new", "self", "try", "catch", "throw",
    "true", "false", "nil", "import", "print",
];

/// A small hand-rolled highlighter (no external syntax-highlighting crate):
/// walks the source by Unicode scalar value (never raw bytes, so it can't
/// split a multi-byte character and panic on a bad string slice) and tags
/// comments, string/f-string literals, numbers, and keywords.
fn highlight_k(dark: bool, code: &str) -> LayoutJob {
    let font_id = FontId::monospace(14.0);
    let (kw, string_c, num_c, com_c, base) = if dark {
        (
            Color32::from_rgb(197, 134, 192),
            Color32::from_rgb(214, 157, 133),
            Color32::from_rgb(181, 206, 168),
            Color32::from_rgb(106, 153, 85),
            Color32::from_rgb(220, 220, 220),
        )
    } else {
        (
            Color32::from_rgb(136, 19, 145),
            Color32::from_rgb(163, 21, 21),
            Color32::from_rgb(9, 134, 88),
            Color32::from_rgb(0, 128, 0),
            Color32::from_rgb(30, 30, 30),
        )
    };
    let fmt = |color: Color32| TextFormat { font_id: font_id.clone(), color, ..Default::default() };

    let mut job = LayoutJob::default();
    let chars: Vec<(usize, char)> = code.char_indices().collect();
    let len = chars.len();
    let byte_len = code.len();
    let byte_at = |pos: usize| if pos < len { chars[pos].0 } else { byte_len };
    let mut idx = 0usize;

    while idx < len {
        let c = chars[idx].1;

        // Line comment.
        if c == '/' && idx + 1 < len && chars[idx + 1].1 == '/' {
            let start = idx;
            while idx < len && chars[idx].1 != '\n' { idx += 1; }
            job.append(&code[byte_at(start)..byte_at(idx)], 0.0, fmt(com_c));
            continue;
        }

        // String / f-string literal (a leading 'f' before the quote is
        // just consumed as part of the preceding identifier scan, so this
        // only needs to handle the quoted body itself).
        if c == '"' || c == '\'' {
            let quote = c;
            let start = idx;
            idx += 1;
            while idx < len {
                let cc = chars[idx].1;
                idx += 1;
                if cc == '\\' && idx < len { idx += 1; continue; }
                if cc == quote { break; }
            }
            job.append(&code[byte_at(start)..byte_at(idx)], 0.0, fmt(string_c));
            continue;
        }

        // Number literal.
        if c.is_ascii_digit() {
            let start = idx;
            while idx < len && chars[idx].1.is_ascii_digit() { idx += 1; }
            if idx < len && chars[idx].1 == '.' && idx + 1 < len && chars[idx + 1].1.is_ascii_digit() {
                idx += 1;
                while idx < len && chars[idx].1.is_ascii_digit() { idx += 1; }
            }
            job.append(&code[byte_at(start)..byte_at(idx)], 0.0, fmt(num_c));
            continue;
        }

        // Identifier / keyword.
        if c.is_alphabetic() || c == '_' {
            let start = idx;
            while idx < len && (chars[idx].1.is_alphanumeric() || chars[idx].1 == '_') { idx += 1; }
            let word = &code[byte_at(start)..byte_at(idx)];
            job.append(word, 0.0, fmt(if KEYWORDS.contains(&word) { kw } else { base }));
            continue;
        }

        // Anything else (punctuation/whitespace) — one char at a time.
        let start = idx;
        idx += 1;
        job.append(&code[byte_at(start)..byte_at(idx)], 0.0, fmt(base));
    }

    job
}

const WELCOME_SOURCE: &str = r#"// Welcome to K — press Run (or F5) to execute.

// 'self' is implicit inside a method (like 'this' in JS/Java) --
// do not declare it as a parameter.
class Animal {
    fn init(name) {
        self.name = name;
    }
    fn speak() {
        return f"{self.name} makes a sound.";
    }
}

class Dog(Animal) {
    fn speak() {
        return f"{self.name} says Woof!";
    }
}

let d = new Dog("Rex");
print(d.speak());

// Matrices are first-class: '@' is matrix multiplication.
let inputs = [[1.5, 0.2]];
let weights = [[0.8, -0.1], [0.4, 0.9]];
let hidden = inputs @ weights;
print("hidden layer:", relu(hidden));

// Closures and recursion both work correctly.
fn fib(n) {
    if n <= 1 { return n; }
    return fib(n - 1) + fib(n - 2);
}
let fibs = [];
for i in range(8) { fibs.append(fib(i)); }
print("fibonacci:", fibs);
"#;

struct KIdeApp {
    code: String,
    output: String,
    file_path: Option<PathBuf>,
    dirty: bool,
    dark_mode: bool,
    last_run_ok: Option<bool>,
    status: String,
}

impl Default for KIdeApp {
    fn default() -> Self {
        Self {
            code: WELCOME_SOURCE.to_owned(),
            output: "Ready. Press Run (or F5) to execute.".to_owned(),
            file_path: None,
            dirty: false,
            dark_mode: true,
            last_run_ok: None,
            status: "Ready.".to_owned(),
        }
    }
}

impl KIdeApp {
    fn display_name(&self) -> String {
        let base = match &self.file_path {
            Some(p) => p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "untitled.k".into()),
            None => "untitled.k".into(),
        };
        if self.dirty { format!("{} •", base) } else { base }
    }

    fn reindent_now(&mut self) {
        self.code = reindent(&self.code);
        self.dirty = true;
        self.status = "Re-indented.".into();
    }

    fn run(&mut self) {
        self.code = reindent(&self.code);
        self.output = run_code(&self.code);
        self.last_run_ok = Some(!looks_like_error(&self.output));
        self.status = if self.last_run_ok == Some(true) { "Ran successfully.".into() } else { "Finished with an error.".into() };
    }

    fn new_file(&mut self) {
        self.code.clear();
        self.file_path = None;
        self.dirty = false;
        self.output.clear();
        self.status = "New file.".into();
    }

    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("K source", &["k"]).pick_file() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => {
                    self.code = contents;
                    self.file_path = Some(path);
                    self.dirty = false;
                    self.status = "Opened.".into();
                }
                Err(e) => self.status = format!("Error opening file: {}", e),
            }
        }
    }

    fn save_file(&mut self) {
        self.code = reindent(&self.code);
        let path = match &self.file_path {
            Some(p) => Some(p.clone()),
            None => rfd::FileDialog::new().add_filter("K source", &["k"]).set_file_name("untitled.k").save_file(),
        };
        self.write_to(path);
    }

    fn save_file_as(&mut self) {
        self.code = reindent(&self.code);
        let path = rfd::FileDialog::new().add_filter("K source", &["k"]).set_file_name("untitled.k").save_file();
        self.write_to(path);
    }

    fn write_to(&mut self, path: Option<PathBuf>) {
        if let Some(path) = path {
            match std::fs::write(&path, &self.code) {
                Ok(_) => {
                    self.file_path = Some(path);
                    self.dirty = false;
                    self.status = "Saved.".into();
                }
                Err(e) => self.status = format!("Error saving file: {}", e),
            }
        }
    }
}

impl eframe::App for KIdeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(if self.dark_mode { egui::Visuals::dark() } else { egui::Visuals::light() });

        let (run_pressed, save_pressed, open_pressed, new_pressed, indent_pressed) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::O),
                i.modifiers.command && i.key_pressed(egui::Key::N),
                i.modifiers.command && i.key_pressed(egui::Key::I),
            )
        });
        if run_pressed { self.run(); }
        if save_pressed { self.save_file(); }
        if open_pressed { self.open_file(); }
        if new_pressed { self.new_file(); }
        if indent_pressed { self.reindent_now(); }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New                Ctrl+N").clicked() { self.new_file(); ui.close_menu(); }
                    if ui.button("Open…              Ctrl+O").clicked() { self.open_file(); ui.close_menu(); }
                    if ui.button("Save                Ctrl+S").clicked() { self.save_file(); ui.close_menu(); }
                    if ui.button("Save As…").clicked() { self.save_file_as(); ui.close_menu(); }
                });
                ui.menu_button("Run", |ui| {
                    if ui.button("Run                       F5").clicked() { self.run(); ui.close_menu(); }
                    if ui.button("Re-indent Code      Ctrl+I").clicked() { self.reindent_now(); ui.close_menu(); }
                    if ui.button("Clear Output").clicked() { self.output.clear(); ui.close_menu(); }
                });
                ui.menu_button("View", |ui| {
                    let label = if self.dark_mode { "Switch to Light Theme" } else { "Switch to Dark Theme" };
                    if ui.button(label).clicked() { self.dark_mode = !self.dark_mode; ui.close_menu(); }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(self.display_name()).monospace());
                });
            });
        });

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("K Language IDE");
                ui.add_space(16.0);
                if ui.button("▶ Run  (F5)").clicked() { self.run(); }
                if ui.button("Re-indent (Ctrl+I)").clicked() { self.reindent_now(); }
                if ui.button("Clear Output").clicked() { self.output.clear(); }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let lines = self.code.lines().count().max(1);
                ui.label(format!("{} lines", lines));
                ui.separator();
                match self.last_run_ok {
                    Some(true) => { ui.colored_label(Color32::from_rgb(80, 200, 120), "● OK"); }
                    Some(false) => { ui.colored_label(Color32::from_rgb(220, 90, 90), "● Error"); }
                    None => { ui.label("● Not run yet"); }
                }
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Source (.k):");
            let dark = self.dark_mode;
            let mut editor_layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| {
                let mut job = highlight_k(dark, text);
                job.wrap.max_width = wrap_width;
                ui.fonts(|f| f.layout_job(job))
            };
            egui::ScrollArea::vertical().id_source("editor").max_height(ui.available_height() * 0.62).show(ui, |ui| {
                let response = ui.add(
                    egui::TextEdit::multiline(&mut self.code)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(20)
                        .lock_focus(true)
                        .code_editor()
                        .layouter(&mut editor_layouter),
                );
                if response.changed() { self.dirty = true; }
            });
            ui.separator();
            ui.label("Output:");
            let output_color = match self.last_run_ok {
                Some(false) => Color32::from_rgb(224, 120, 120),
                _ if dark => Color32::from_rgb(210, 210, 210),
                _ => Color32::from_rgb(30, 30, 30),
            };
            egui::ScrollArea::vertical().id_source("output").show(ui, |ui| {
                let mut output_layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    let mut job = LayoutJob::single_section(
                        text.to_owned(),
                        TextFormat { font_id: FontId::monospace(13.5), color: output_color, ..Default::default() },
                    );
                    job.wrap.max_width = wrap_width;
                    ui.fonts(|f| f.layout_job(job))
                };
                ui.add(
                    egui::TextEdit::multiline(&mut self.output.as_str())
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(10)
                        .layouter(&mut output_layouter),
                );
            });
        });
    }
}
