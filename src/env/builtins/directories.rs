//! Working directory: `cd`, `pwd`, and the directory stack (`pushd`, `popd`, `dirs`).

use crate::env::scope::Environment;
use crate::error::Result;
use std::env;
use std::path::PathBuf;

pub fn builtin_cd(env: &mut Environment, args: &[String]) -> Result<i32> {
    let target_path = if args.len() > 1 {
        if args[1] == "-" {
            match env.get_var("OLDPWD") {
                Some(old) => {
                    println!("{}", old);
                    PathBuf::from(old)
                }
                None => {
                    eprintln!("rush: cd: OLDPWD not set");
                    return Ok(1);
                }
            }
        } else {
            PathBuf::from(&args[1])
        }
    } else {
        match env.get_var("HOME") {
            Some(h) => PathBuf::from(h),
            None => {
                eprintln!("rush: cd: HOME not set");
                return Ok(1);
            }
        }
    };

    let current_pwd = env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    if let Err(e) = env::set_current_dir(&target_path) {
        eprintln!("rush: cd: {}: {}", target_path.display(), e);
        return Ok(1);
    }

    let new_pwd = env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    env.set_var("OLDPWD", &current_pwd, true);
    env.set_var("PWD", &new_pwd, true);

    Ok(0)
}

pub fn builtin_pwd(_env: &mut Environment, _args: &[String]) -> Result<i32> {
    match env::current_dir() {
        Ok(path) => {
            println!("{}", path.display());
            Ok(0)
        }
        Err(e) => {
            eprintln!("rush: pwd: error retrieving current directory: {}", e);
            Ok(1)
        }
    }
}

pub fn builtin_pushd(env: &mut Environment, args: &[String]) -> Result<i32> {
    let current_pwd = match env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("rush: pushd: {}", e);
            return Ok(1);
        }
    };

    if args.len() < 2 {
        eprintln!("rush: pushd: no other directory");
        return Ok(1);
    }

    let target = PathBuf::from(&args[1]);
    if let Err(e) = env::set_current_dir(&target) {
        eprintln!("rush: pushd: {}: {}", target.display(), e);
        return Ok(1);
    }

    env.push_dir(current_pwd);
    let new_pwd = env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    env.set_var("PWD", &new_pwd, true);
    builtin_dirs(env, &[])
}

pub fn builtin_popd(env: &mut Environment, _args: &[String]) -> Result<i32> {
    if let Some(target) = env.pop_dir() {
        if let Err(e) = env::set_current_dir(&target) {
            eprintln!("rush: popd: {}: {}", target.display(), e);
            return Ok(1);
        }
        let new_pwd = env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        env.set_var("PWD", &new_pwd, true);
        builtin_dirs(env, &[])
    } else {
        eprintln!("rush: popd: directory stack empty");
        Ok(1)
    }
}

pub fn builtin_dirs(env: &mut Environment, _args: &[String]) -> Result<i32> {
    let current = env::current_dir().unwrap_or_default();
    let mut dirs_list = vec![current.display().to_string()];
    for dir in env.get_dir_stack().iter().rev() {
        dirs_list.push(dir.display().to_string());
    }
    println!("{}", dirs_list.join(" "));
    Ok(0)
}
