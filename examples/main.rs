use rush::env::Environment;
use rush::exec::eval_command_list;
use rush::lexer::Lexer;
use rush::parser::Parser;

fn main() {
    let mut env = Environment::new();
    let script = "echo 'Hello from rush example!'; X=42; echo \"X is $X\"";
    let lexer = Lexer::new(script);
    let mut parser = Parser::new(lexer);
    let ast = parser.parse_command_list().unwrap();
    let _ = eval_command_list(&mut env, &ast);
}
