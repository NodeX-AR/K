// Hides the terminal window on Windows when launching the GUI. This makes
// release builds a "windows subsystem" binary, which means Windows never
// attaches a console to it at all — so `k -h`, `k -v`, and `k repl` would
// otherwise print/read nothing when run from PowerShell or cmd.exe. The
// `attach_console()` call below fixes that by reattaching to whatever
// console launched the process (a no-op, harmlessly, for `k gui` or when
// double-clicked with no parent console).
#![cfg_attr(all(not(debug_assertions), feature = "gui"), windows_subsystem = "windows")]

mod ast;
mod lexer;
mod parser;
mod chunk;
mod value;
mod compiler;
mod vm;
#[cfg(feature = "gui")]
mod idle;

use std::env;
use std::fs;
use std::io::{self, Write};

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// A handful of ANSI codes for the terminal CLI. Kept minimal and manual
// (no extra crate) since we only ever need a handful of fixed colors.
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GREEN: &str = "\x1b[32m";
    pub const RED: &str = "\x1b[31m";
    pub const YELLOW: &str = "\x1b[33m";
}

/// On Windows, a `windows_subsystem = "windows"` binary starts with no
/// console attached, so stdio goes nowhere even when launched from an
/// existing terminal. This reattaches to the parent process's console (the
/// shell that ran `k`) and repoints stdin/stdout/stderr at it, so the CLI
/// paths (`-h`, `-v`, `repl`, running a script) behave like a normal
/// console program again. `k gui` still opens its own window either way.
///
/// Safe to call unconditionally: if there's no parent console to attach to
/// (e.g. launched by double-clicking, or already has one in a debug
/// build), `AttachConsole` simply fails and this is a no-op.
#[cfg(windows)]
fn attach_console() {
    use std::ffi::CString;
    use std::ptr::null_mut;
    use winapi::um::fileapi::{CreateFileA, OPEN_EXISTING};
    use winapi::um::handleapi::INVALID_HANDLE_VALUE;
    use winapi::um::processenv::SetStdHandle;
    use winapi::um::winbase::{STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
    use winapi::um::wincon::{AttachConsole, ATTACH_PARENT_PROCESS};
    use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE};

    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return; // no parent console to attach to — leave as-is
        }

        if let Ok(conout) = CString::new("CONOUT$") {
            let handle = CreateFileA(
                conout.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if handle != INVALID_HANDLE_VALUE {
                SetStdHandle(STD_OUTPUT_HANDLE, handle);
                SetStdHandle(STD_ERROR_HANDLE, handle);
            }
        }

        if let Ok(conin) = CString::new("CONIN$") {
            let handle = CreateFileA(
                conin.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                0,
                null_mut(),
            );
            if handle != INVALID_HANDLE_VALUE {
                SetStdHandle(STD_INPUT_HANDLE, handle);
            }
        }
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() {
    attach_console();
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("gui") => launch_gui(),
        Some("repl") => repl(),
        Some("--version") | Some("-v") => {
            println!("K Language v{}", VERSION);
        }
        Some("--help") | Some("-h") => print_help(),
        Some("fmt") => {
            let (check_mode, path) = if args.get(2).map(|s| s.as_str()) == Some("--check") {
                (true, args.get(3))
            } else {
                (false, args.get(2))
            };
            match path {
                Some(p) => cmd_fmt(p, check_mode),
                None => { eprintln!("Usage: k fmt [--check] <file.k>"); std::process::exit(1); }
            }
        }
        Some("test") => {
            match args.get(2) {
                Some(p) => cmd_test(p),
                None => { eprintln!("Usage: k test <file_or_directory>"); std::process::exit(1); }
            }
        }
        Some(filename) if !filename.starts_with('-') => {
            match fs::read_to_string(filename) {
                Ok(code) => {
                    let script_args: Vec<String> = args.iter().skip(2).cloned().collect();
                    let output = run_code(&code, script_args);
                    print!("{}", output);
                    if output.starts_with("Lex error") || output.starts_with("Parse error")
                        || output.starts_with("Compile error") || output.contains("Uncaught error") {
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Error reading file '{}': {}", filename, e);
                    std::process::exit(1);
                }
            }
        }
        // No arguments and no file: drop into the interactive REPL — if
        // you're typing `k` in a terminal, you want a terminal, not a
        // window. Use `k gui` for the graphical IDE.
        None => repl(),
        _ => print_help(),
    }
}

/// `k gui` is only wired up when the binary was built with the `gui`
/// Cargo feature (on by default; a CLI/container build can opt out with
/// `--no-default-features` to drop eframe/rfd/image entirely and shrink
/// the binary). Off-feature, this prints a clear message instead of the
/// subcommand silently not existing.
#[cfg(feature = "gui")]
fn launch_gui() { idle::launch(); }
#[cfg(not(feature = "gui"))]
fn launch_gui() {
    eprintln!("This build was compiled without the 'gui' feature, so there's no graphical IDE available.");
    eprintln!("Rebuild with `cargo build --release --features gui` (the default build already includes it)");
    eprintln!("to get `k gui` back, or use `k repl` / `k <file.k>` from here.");
    std::process::exit(1);
}

fn cmd_fmt(path: &str, check_mode: bool) {
    let code = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => { eprintln!("Error reading '{}': {}", path, e); std::process::exit(1); }
    };
    let formatted = reindent(&code);
    let changed = formatted != code;
    if check_mode {
        if changed { println!("{}: would reformat", path); std::process::exit(1); }
        println!("{}: already formatted", path);
    } else if changed {
        match fs::write(path, &formatted) {
            Ok(_) => println!("{}: reformatted", path),
            Err(e) => { eprintln!("Error writing '{}': {}", path, e); std::process::exit(1); }
        }
    } else {
        println!("{}: already formatted", path);
    }
}

/// Re-indents from scratch based on `{`/`}` nesting alone, ignoring braces
/// inside strings/comments. K has no significant whitespace, so this can
/// only change how a script *looks*, never what it does. (Same logic as
/// the GUI's auto-reindent, duplicated here in a few lines rather than
/// shared, so `k fmt` still works in a build compiled without the `gui`
/// feature.)
fn reindent(code: &str) -> String {
    let mut depth: i32 = 0;
    let mut out_lines: Vec<String> = Vec::new();
    for raw_line in code.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() { out_lines.push(String::new()); continue; }
        let this_depth = if trimmed.starts_with('}') { (depth - 1).max(0) } else { depth };
        out_lines.push(format!("{}{}", "    ".repeat(this_depth as usize), trimmed));
        depth = (depth + net_brace_change(trimmed)).max(0);
    }
    out_lines.join("\n")
}
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
            '/' if chars.peek() == Some(&'/') => break,
            '{' => net += 1,
            '}' => net -= 1,
            _ => {}
        }
    }
    net
}

/// Runs every `*_test.k` file under `path` (or `path` itself, if it's a
/// single file) and reports pass/fail. A file "passes" if it runs to
/// completion without a lex/parse/compile error or an uncaught runtime
/// error — `assert()` failures surface as exactly that, so tests are just
/// ordinary scripts full of `assert(...)` calls. Name a file
/// `*_shouldfail_test.k` to flip that: it passes only if running it DOES
/// produce an error — for asserting things like "assigning to a const is a
/// compile error", which can't be caught from inside the script itself.
fn cmd_test(path: &str) {
    let files = collect_test_files(path);
    if files.is_empty() {
        println!("No test files found (looking for '{}', or '*_test.k' under it if it's a directory)", path);
        return;
    }
    let (mut passed, mut failed) = (0, 0);
    for file in &files {
        let expect_fail = file.file_name().and_then(|n| n.to_str()).map(|n| n.contains("_shouldfail_")).unwrap_or(false);
        match fs::read_to_string(file) {
            Ok(code) => {
                let output = run_code(&code, Vec::new());
                let errored = output.starts_with("Lex error") || output.starts_with("Parse error")
                    || output.starts_with("Compile error") || output.contains("Uncaught error");
                let ok = if expect_fail { errored } else { !errored };
                if ok {
                    passed += 1;
                    println!("PASS {}", file.display());
                } else {
                    failed += 1;
                    println!("FAIL {}{}", file.display(), if expect_fail { " (expected an error, but it ran clean)" } else { "" });
                    for line in output.lines() { println!("     {}", line); }
                }
            }
            Err(e) => { failed += 1; println!("FAIL {} (could not read: {})", file.display(), e); }
        }
    }
    println!("\n{} passed, {} failed", passed, failed);
    if failed > 0 { std::process::exit(1); }
}

fn collect_test_files(path: &str) -> Vec<std::path::PathBuf> {
    let p = std::path::Path::new(path);
    let mut out = Vec::new();
    if p.is_file() {
        out.push(p.to_path_buf());
    } else if p.is_dir() {
        if let Ok(entries) = fs::read_dir(p) {
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort();
            for entry in paths {
                if entry.is_file() && entry.file_name().and_then(|n| n.to_str()).map(|n| n.ends_with("_test.k")).unwrap_or(false) {
                    out.push(entry);
                }
            }
        }
    }
    out
}

fn print_help() {
    println!("K Language v{}", VERSION);
    println!("Usage: k [OPTIONS] [FILE]");
    println!();
    println!("Commands:");
    println!("  (no args)        Start the interactive REPL (same as 'k repl')");
    println!("  gui              Launch the K IDE (graphical editor)");
    println!("  repl             Start an interactive REPL shell");
    println!("  <file.k> [args]  Run a K script file, passing extra args to args()");
    println!("  fmt [--check] <file.k>   Reformat a file's indentation in place");
    println!("                            (--check: report only, exit nonzero if it would change)");
    println!("  test <file_or_dir>        Run *_test.k files and report pass/fail");
    println!();
    println!("Options:");
    println!("  -h, --help       Show this help message");
    println!("  -v, --version    Show version information");
    println!();
    println!("Examples:");
    println!("  k gui            # Open the IDE");
    println!("  k repl           # Start interactive shell");
    println!("  k script.k       # Run script.k");
    println!("  k fmt myfile.k   # Reformat myfile.k in place");
    println!("  k test tests/    # Run every *_test.k file under tests/");
    println!();
    println!("REPL: arrow-key history, Ctrl+R search, multi-line input, and");
    println!("      :help/:load/:vars/:clear/:exit commands once inside.");
}

/// Lex, parse, compile to bytecode, and run — every stage returns a
/// Result; a bad script prints a message and stops, it never panics.
fn run_code(code: &str, script_args: Vec<String>) -> String {
    let tokens = match lexer::tokenize(code) {
        Ok(t) => t,
        Err(e) => return format!("Lex error: {}\n", e),
    };
    let stmts = match parser::parse(tokens) {
        Ok(s) => s,
        Err(e) => return format!("Parse error: {}\n", e),
    };
    let function = match compiler::Compiler::compile_program(&stmts) {
        Ok(f) => f,
        Err(e) => return format!("Compile error: {}\n", e),
    };
    let mut machine = vm::VM::new();
    machine.script_args = script_args;
    machine.run_program(function)
}

/// Number of still-open `{`/`(`/`[` in `src`, ignoring brackets inside
/// string literals or comments — good enough for deciding whether the
/// REPL should keep reading a continuation line, not a full lexer pass.
fn unclosed_brackets(src: &str) -> i32 {
    let mut depth = 0i32;
    let mut chars = src.chars().peekable();
    let mut in_string = false;
    let mut string_quote = ' ';
    while let Some(c) = chars.next() {
        if in_string {
            if c == '\\' { chars.next(); continue; }
            if c == string_quote { in_string = false; }
            continue;
        }
        match c {
            '"' | '\'' => { in_string = true; string_quote = c; }
            '/' if chars.peek() == Some(&'/') => {
                // line comment: skip to end of this line
                while let Some(&nc) = chars.peek() {
                    if nc == '\n' { break; }
                    chars.next();
                }
            }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth
}

const REPL_HELP: &str = "\
Commands:
  :help              Show this help
  :load <file.k>     Load and run a .k file in this session
  :vars              List current global variables
  :clear             Clear the screen
  :exit, :q          Quit (Ctrl+D also works)

Multi-line input: if a line leaves an open '{', '(' or '[', the REPL
keeps reading (shown with a '...' prompt) until it's balanced.";

/// Lex, parse, and compile `code`, then run it on an existing VM so
/// variables/functions/classes defined on earlier lines stay in scope.
fn run_program_from_source(machine: &mut vm::VM, code: &str) -> String {
    let tokens = match lexer::tokenize(code) {
        Ok(t) => t,
        Err(e) => return format!("Lex error: {}\n", e),
    };
    let stmts = match parser::parse(tokens) {
        Ok(s) => s,
        Err(e) => return format!("Parse error: {}\n", e),
    };
    let function = match compiler::Compiler::compile_program(&stmts) {
        Ok(f) => f,
        Err(e) => return format!("Compile error: {}\n", e),
    };
    machine.run_program(function)
}

fn repl() {
    let mut machine = vm::VM::new();
    println!(
        "{b}K Language REPL v{v}{r} — {d}:help for commands, Ctrl+D or :exit to quit{r}",
        b = ansi::BOLD, v = VERSION, r = ansi::RESET, d = ansi::DIM
    );

    let mut rl = match DefaultEditor::new() {
        Ok(rl) => rl,
        Err(e) => {
            eprintln!("Could not start line editor ({}), falling back to plain input.", e);
            return repl_plain(machine);
        }
    };
    let history_path = dirs_history_path();
    if let Some(p) = &history_path { let _ = rl.load_history(p); }

    'outer: loop {
        let mut buffer = String::new();
        let mut prompt = format!("{}k>{} ", ansi::CYAN, ansi::RESET);
        loop {
            match rl.readline(&prompt) {
                Ok(line) => {
                    if buffer.is_empty() {
                        let trimmed = line.trim();
                        if trimmed == ":exit" || trimmed == ":q" { break 'outer; }
                        if trimmed == ":help" { println!("{}", REPL_HELP); continue 'outer; }
                        if trimmed == ":clear" { print!("\x1b[2J\x1b[H"); io::stdout().flush().ok(); continue 'outer; }
                        if trimmed == ":vars" {
                            if machine.globals.is_empty() {
                                println!("{}(no globals yet){}", ansi::DIM, ansi::RESET);
                            } else {
                                let mut names: Vec<&String> = machine.globals.keys().collect();
                                names.sort();
                                for name in names {
                                    println!("  {} = {}", name, crate::value::to_display(&machine.globals[name]));
                                }
                            }
                            continue 'outer;
                        }
                        if let Some(path) = trimmed.strip_prefix(":load ") {
                            match fs::read_to_string(path.trim()) {
                                Ok(code) => print!("{}", run_program_from_source(&mut machine, &code)),
                                Err(e) => println!("{}Error reading '{}': {}{}", ansi::RED, path.trim(), e, ansi::RESET),
                            }
                            continue 'outer;
                        }
                        if trimmed.is_empty() { continue 'outer; }
                    }
                    if !buffer.is_empty() { buffer.push('\n'); }
                    buffer.push_str(&line);
                    if unclosed_brackets(&buffer) > 0 {
                        prompt = format!("{}...{} ", ansi::DIM, ansi::RESET);
                        continue;
                    }
                    break;
                }
                Err(ReadlineError::Interrupted) => { buffer.clear(); continue 'outer; } // Ctrl+C: abandon this line
                Err(ReadlineError::Eof) => break 'outer, // Ctrl+D
                Err(_) => break 'outer,
            }
        }

        rl.add_history_entry(buffer.as_str()).ok();
        let out = run_program_from_source(&mut machine, &buffer);
        if out.starts_with("Lex error")
            || out.starts_with("Parse error")
            || out.starts_with("Compile error")
            || out.contains("Traceback")
            || out.contains("Runtime error")
        {
            print!("{}{}{}", ansi::RED, out, ansi::RESET);
        } else {
            print!("{}", out);
        }
        io::stdout().flush().ok();
    }

    if let Some(p) = &history_path { let _ = rl.save_history(p); }
    println!("{}Goodbye.{}", ansi::YELLOW, ansi::RESET);
}

/// Fallback REPL (no line editing/history) used only if the terminal
/// doesn't support rustyline's raw mode, so the REPL never just refuses
/// to start.
fn repl_plain(mut machine: vm::VM) {
    loop {
        print!("k> ");
        if io::stdout().flush().is_err() { break; }
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let line = line.trim();
        if line == ":exit" || line == ":q" { break; }
        if line.is_empty() { continue; }
        print!("{}", run_program_from_source(&mut machine, line));
        io::stdout().flush().ok();
    }
}

fn dirs_history_path() -> Option<std::path::PathBuf> {
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))?;
    Some(std::path::PathBuf::from(home).join(".k_history"))
}
