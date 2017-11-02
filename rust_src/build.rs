use std::env;
use std::fs;
use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Eq, PartialEq)]
enum ParseState {
    Looking,
    IgnoreComments,
    NoMangleFn,
    LispFn(Option<String>),
}

fn get_function_name(line: &str) -> Option<String> {
    if let Some(pos) = line.find('(') {
        if let Some(fnpos) = line.find("fn ") {
            return Some((&line[(fnpos + 3)..pos]).to_string());
        }
    }

    None
}

static C_NAME: &'static str = "c_name = \"";

fn c_exports_for_module<R>(
    modname: &str,
    mut out_file: &File,
    in_file: R,
) -> Result<bool, io::Error>
where
    R: BufRead,
{
    let mut parse_state = ParseState::Looking;
    let mut has_lisp_fns = false;
    let mut exported: Vec<String> = Vec::new();
    let mut has_include = false;

    for line in in_file.lines() {
        let line = line?;

        match parse_state {
            ParseState::Looking => {
                if line.starts_with("#[lisp_fn") {
                    if let Some(begin) = line.find(C_NAME) {
                        let input = line.to_string();
                        let start = begin + C_NAME.len();
                        let end = &input[start..].find('"').unwrap() + start;
                        let name = (&line[start..end]).to_string();
                        // Ignore macros, nothing we can do with them
                        if !name.starts_with('$') {
                            // even though we do not need to parse the
                            // next line of the file this keeps all of the
                            // lisp_fn code in one place.
                            parse_state = ParseState::LispFn(Some(name));
                        }
                    } else {
                        parse_state = ParseState::LispFn(None);
                    }
                    has_lisp_fns = true;
                } else if line.starts_with("#[no_mangle]") {
                    parse_state = ParseState::NoMangleFn;
                } else if line.starts_with("/*") {
                    if !line.ends_with("*/") {
                        parse_state = ParseState::IgnoreComments;
                    }
                } else if line.starts_with("include!(concat!(env!(\"OUT_DIR\"),") {
                    has_include = true;
                }
            }

            ParseState::IgnoreComments => if line.starts_with("*/") || line.ends_with("*/") {
                parse_state = ParseState::Looking;
            },

            ParseState::LispFn(name) => {
                if line.starts_with("pub") || line.starts_with("fn") {
                    if let Some(func) = name.or_else(|| get_function_name(&line)) {
                        write!(out_file, "pub use {}::F{};\n", modname, func)?;
                        exported.push(func);
                    }
                }

                parse_state = ParseState::Looking;
            }

            // export public #[no_mangle] functions
            ParseState::NoMangleFn => {
                if line.starts_with("pub") {
                    if let Some(func) = get_function_name(&line) {
                        write!(out_file, "pub use {}::{};\n", modname, func)?;
                    }
                }

                parse_state = ParseState::Looking;
            }
        };
    }

    if has_lisp_fns {
        let path =
            PathBuf::from(env::var("OUT_DIR").unwrap()).join([modname, "_exports.rs"].concat());
        let mut exports_file = File::create(path)?;

        write!(
            exports_file,
            "export_lisp_fns! {{ {} }}",
            exported.join(", ")
        )?;

        if !has_include {
            panic!(
                [
                    modname,
                    ".rs is missing the required include for lisp_fn exports"
                ].concat()
            );
        }
    }

    Ok(has_lisp_fns)
}

fn ignore(path: &str) -> bool {
    path == "" || path.starts_with('.') || path == "lib.rs"
}

fn generate_c_exports() -> Result<(), io::Error> {
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("c_exports.rs");
    let mut out_file = File::create(out_path)?;

    let mut modules: Vec<String> = Vec::new();

    let in_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("src");
    for entry in fs::read_dir(in_path)? {
        let entry = entry?;
        let mut mod_path = entry.path();

        if ignore(mod_path.file_name().unwrap().to_str().unwrap()) {
            continue;
        }

        let mut name: Option<String> = None;

        if mod_path.is_dir() {
            name = Some(
                mod_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map_or_else(|| panic!("Cannot understand string"), |s| s.to_string()),
            );
            mod_path = mod_path.join("mod.rs");
        } else if let Some(ext) = mod_path.extension() {
            if ext == "rs" {
                name = Some(
                    mod_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map_or_else(|| panic!("Cannot understand string"), |s| s.to_string()),
                );
            }
        }

        if let Some(modname) = name {
            let path = mod_path
                .to_str()
                .map_or_else(|| panic!("Cannot understand string"), |s| s.to_string());

            let fp = match File::open(Path::new(&path)) {
                Ok(f) => f,

                Err(e) => {
                    eprintln!("Failed to open {}", path);
                    return Err(e);
                }
            };

            if c_exports_for_module(&modname, &out_file, BufReader::new(fp))? {
                modules.push(modname);
            }
        }
    }

    write!(out_file, "\n")?;

    write!(
        out_file,
        "#[no_mangle]\npub extern \"C\" fn rust_init_syms() {{\n"
    )?;
    for module in modules {
        write!(out_file, "    {}::rust_init_syms();\n", module)?;
    }
    // Add this one by hand.
    write!(out_file, "    floatfns::rust_init_extra_syms();\n")?;
    write!(out_file, "}}\n")?;

    Ok(())
}

fn main() {
    if let Err(e) = generate_c_exports() {
        eprintln!("Errors occurred: {}", e);
    }
}
