//! Process and signal control: `trap`, `umask`, `wait`, `kill`.

use crate::env::scope::Environment;
use crate::error::Result;

pub fn builtin_trap(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        for (sig, handler) in env.get_traps() {
            println!("trap -- {:?} {}", handler, sig);
        }
        return Ok(0);
    }

    let handler = &args[1];
    for sig in &args[2..] {
        env.set_trap(sig, handler);
    }
    Ok(0)
}

pub fn builtin_umask(_env: &mut Environment, args: &[String]) -> Result<i32> {
    use nix::sys::stat::{Mode, umask};
    if args.len() < 2 {
        let current = umask(Mode::empty());
        umask(current);
        println!("{:04o}", current.bits());
        return Ok(0);
    }

    if let Ok(mask_val) = u32::from_str_radix(&args[1], 8)
        && let Some(mode) = Mode::from_bits(mask_val as nix::sys::stat::mode_t)
    {
        umask(mode);
    }
    Ok(0)
}

pub fn builtin_wait(env: &mut Environment, args: &[String]) -> Result<i32> {
    use nix::sys::wait::waitpid;
    use nix::unistd::Pid;
    if args.len() < 2 {
        if let Some(bg_pid) = env.last_bg_pid {
            let pid = Pid::from_raw(bg_pid as i32);
            let _ = waitpid(pid, None);
        }
        return Ok(0);
    }

    for arg in &args[1..] {
        if let Ok(pid_i32) = arg.parse::<i32>() {
            let pid = Pid::from_raw(pid_i32);
            let _ = waitpid(pid, None);
        }
    }
    Ok(0)
}

pub fn builtin_kill(_env: &mut Environment, args: &[String]) -> Result<i32> {
    use nix::sys::signal::{self, Signal};
    use nix::unistd::Pid;
    use std::str::FromStr;

    if args.len() < 2 {
        eprintln!("rush: kill: usage: kill [-sig] pid...");
        return Ok(1);
    }

    let mut sig = Signal::SIGTERM;
    let mut pids_start = 1;

    if args[1].starts_with('-') {
        let sig_name = args[1].trim_start_matches('-').to_uppercase();
        if let Ok(parsed_sig) = Signal::from_str(&sig_name) {
            sig = parsed_sig;
        } else if let Ok(sig_num) = sig_name.parse::<i32>()
            && let Ok(parsed_sig) = Signal::try_from(sig_num)
        {
            sig = parsed_sig;
        }
        pids_start = 2;
    }

    for arg in &args[pids_start..] {
        if let Ok(pid_i32) = arg.parse::<i32>() {
            let pid = Pid::from_raw(pid_i32);
            if let Err(e) = signal::kill(pid, sig) {
                eprintln!("rush: kill: ({}) - {}", pid, e);
                return Ok(1);
            }
        }
    }

    Ok(0)
}
