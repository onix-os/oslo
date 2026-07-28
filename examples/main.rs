use rush::env::Environment;
use rush::exec::eval_command_list;
use rush::parser::parse_bash_script;

fn main() {
    let mut env = Environment::new();
    let script = "echo 'Hello from rush example!'; X=42; echo \"X is $X\"";
    let ast = parse_bash_script(script).expect("the example script must parse");
    let _ = eval_command_list(&mut env, &ast);
}
