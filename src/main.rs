use std::process::ExitCode;

fn main() -> ExitCode {
    let mut json = false;
    let mut path = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => json = true,
            _ => path = Some(arg),
        }
    }
    let Some(path) = path else {
        eprintln!("usage: notist [--json] <file.not>");
        return ExitCode::FAILURE;
    };
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(err) => {
            eprintln!("{path}: {err}");
            return ExitCode::FAILURE;
        }
    };
    if json {
        println!("{}", notist::cst_json::analyze_json(&src));
    } else {
        let parse = notist::parser::parse(&src);
        println!("{:#?}", parse.syntax());
    }
    ExitCode::SUCCESS
}
