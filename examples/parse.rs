use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: parse <input-file> <output-dir>");
        return ExitCode::from(2);
    }
    match tokimo_package_fileparser::parse(&args[1], &args[2]) {
        Ok(out) => {
            println!("{}", out.to_json_pretty());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
