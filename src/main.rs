pub mod tokenizer;
pub mod utils;
pub mod parser;
pub mod cforge;

use std::env;

pub use utils::*;

#[tokio::main]
async fn main() {
    // let c = fs::read("./test.crs").unwrap();
    // let c = String::from_utf8(c).unwrap();

    // let mut tokenizer = Tokenizer::new(c);

    // let tokens = tokenizer.tokenize();

    // // println!("{:?}", tokens.iter().map(|t| t.to_string()).collect::<Vec<String>>());

    env::set_var("CFORGE_VERSION", "0.1.0");
    env::set_var("COPPER_VERSION", "0.1.0");
    env::set_var("COPPER_PATH", env::current_exe().unwrap().parent().unwrap().to_str().unwrap());
    cforge::print();
    cforge::compile(vec!["./main.crs".to_string()]);
    cforge::generate_toml().await;
    cforge::run();
}
