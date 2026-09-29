use std::process::ExitCode;

fn main() -> ExitCode {
    let mut cst = false;
    let mut json = false;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--cst" => cst = true,
            "--json" => json = true,
            _ => path = Some(arg),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: notist [--cst | --json] <file.not>");
        return ExitCode::FAILURE;
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("{path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    let parse = notist::parser::parse(&src);
    if json {
        println!("{}", notist::cst_json::analyze_json(&src));
    } else if cst {
        println!("{:#?}", parse.syntax());
    } else {
        print!("{}", notist::dump_str(&src));
    }
    ExitCode::SUCCESS
}
