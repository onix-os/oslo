use rush::env::Environment;
use rush::error::{Result, ShellError};
use rush::exec::{JobManager, eval_command_list};
use rush::interactive::RushHelper;
use rush::lexer::Lexer;
use rush::lua::LuaEngine;
use rush::parser::Parser;
use rustyline::Editor;
use rustyline::error::ReadlineError;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        if args[1] == "-c" && args.len() > 2 {
            // Run inline command
            let mut env = Environment::new();
            match run_string(&mut env, &args[2]) {
                // The shell's exit status is that of the last command it ran.
                Ok(status) => std::process::exit(status),
                Err(e) => handle_exit_error(e),
            }
        } else if args[1] == "--lua-script" && args.len() > 2 {
            // Run Lua script directly
            let env = Arc::new(Mutex::new(Environment::new()));
            let lua = LuaEngine::new().expect("Failed to initialize Lua");
            let _ = lua.setup_bindings(env);
            if let Err(e) = lua.load_file(&args[2]) {
                eprintln!("rush: lua error: {}", e);
                std::process::exit(1);
            }
            return;
        } else if !args[1].starts_with('-') {
            // Run script file
            let mut env = Environment::new();
            env.set_positional(args[2..].to_vec());
            match fs::read_to_string(&args[1]) {
                Ok(script) => match run_string(&mut env, &script) {
                    Ok(status) => std::process::exit(status),
                    Err(e) => handle_exit_error(e),
                },
                Err(_) => {
                    eprintln!("rush: {}: No such file or directory", args[1]);
                    std::process::exit(127);
                }
            }
        }
    }

    // Run interactive REPL
    run_repl();
}

fn handle_exit_error(err: ShellError) {
    match err {
        ShellError::Exit(code) => std::process::exit(code),
        e => {
            eprintln!("rush: {}", e);
            std::process::exit(1);
        }
    }
}

fn run_string(env: &mut Environment, input: &str) -> Result<i32> {
    let result = if let Ok(ast) = rush::parser::brush_adapter::parse_bash_script(input) {
        eval_command_list(env, &ast)
    } else {
        let lexer = Lexer::new(input);
        let mut parser = Parser::new(lexer);
        let ast = parser.parse_command_list()?;
        eval_command_list(env, &ast)
    };

    absorb_loop_control(result)
}

/// `break`, `continue` and `return` outside any loop or function are a no-op, not an error.
///
/// They unwind as errors so nested command lists can pass them up; if nothing catches one it has
/// reached the top level, where bash silently ignores it rather than printing a diagnostic.
fn absorb_loop_control(result: Result<i32>) -> Result<i32> {
    match result {
        Err(ShellError::Break(_)) | Err(ShellError::Continue(_)) => Ok(0),
        Err(ShellError::Return(code)) => Ok(code),
        other => other,
    }
}

fn run_repl() {
    let env_struct = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Failed to initialize Lua engine");
    let _ = lua.setup_bindings(Arc::clone(&env_struct));

    // Try loading ~/.config/rush/init.lua
    if let Some(home) = env::var_os("HOME") {
        let init_path = PathBuf::from(home).join(".config/rush/init.lua");
        if init_path.exists() {
            let _ = lua.load_file(init_path.to_str().unwrap());
        }
    }

    let config = rustyline::Config::builder()
        .auto_add_history(true)
        .completion_type(rustyline::CompletionType::Circular)
        .build();

    let mut rl = Editor::with_config(config).expect("Failed to initialize line editor");
    let helper = RushHelper::new(Arc::clone(&env_struct));
    rl.set_helper(Some(helper));

    let history_path = env::var_os("HOME").map(|h| PathBuf::from(h).join(".rush_history"));

    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    let mut jobs = JobManager::new();
    jobs.setup_signals();

    println!("rush v0.1.0 - POSIX Compatible Shell with Lua & Fish-style Features");
    println!("Type 'exit' or Ctrl-D to exit.");

    let mut last_status = 0;

    loop {
        let left_prompt = if let Some(p) = lua.render_prompt() {
            p
        } else {
            rush::interactive::prompt::render_default_left_prompt(last_status)
        };

        let readline = rl.readline(&left_prompt);
        match readline {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(trimmed);

                let mut env_guard = env_struct.lock().unwrap();
                let res = if let Ok(ast) = rush::parser::brush_adapter::parse_bash_script(trimmed) {
                    eval_command_list(&mut env_guard, &ast)
                } else {
                    let lexer = Lexer::new(trimmed);
                    let mut parser = Parser::new(lexer);
                    if let Ok(ast) = parser.parse_command_list() {
                        eval_command_list(&mut env_guard, &ast)
                    } else {
                        Err(ShellError::SyntaxError(
                            "Failed to parse command".to_string(),
                        ))
                    }
                };
                let res = absorb_loop_control(res);

                match res {
                    Ok(status) => {
                        last_status = status;
                    }
                    Err(ShellError::Exit(code)) => {
                        if let Some(ref path) = history_path {
                            let _ = rl.save_history(path);
                        }
                        std::process::exit(code);
                    }
                    Err(err) => {
                        eprintln!("rush: {}", err);
                        last_status = 1;
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
            }
            Err(ReadlineError::Eof) => {
                println!("exit");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }
}
