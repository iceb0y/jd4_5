use std::collections::HashMap;
use std::path::PathBuf;
use linear_map::LinearMap;
use serde_yaml;
use shlex;
use compile::{BinaryCompiler, Compiler, Interpreter};

pub struct Registry {
    compilers: HashMap<String, Box<Compiler>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct CompilerConfig {
    #[serde(rename="type")]
    kind: String,
    compiler_file: Option<PathBuf>,
    compiler_args: Option<String>,
    code_file: PathBuf,
    execute_file: PathBuf,
    execute_args: String,
}

impl Registry {
    pub fn builtin() -> &'static Registry {
        lazy_static! {
            static ref BUILTIN_REGISTRY: Registry = Registry {
                compilers: parse_compilers_yaml(
                    include_bytes!("data/compilers.yaml")),
            };
        }
        &BUILTIN_REGISTRY
    }

    pub fn get_compiler(&self, id: &str) -> Option<&Compiler> {
        self.compilers.get(id).map(Box::as_ref)
    }
}

fn parse_compilers_yaml(v: &[u8]) -> HashMap<String, Box<Compiler>> {
    let configs: LinearMap<String, CompilerConfig> =
        serde_yaml::from_slice(v).unwrap();
    configs.into_iter().map(|(id, config)| {
        (id, match config.kind.as_ref() {
            "compiler" => {
                Box::new(BinaryCompiler::new(
                    config.compiler_file.unwrap(),
                    shlex::split(&config.compiler_args.as_ref().unwrap())
                        .unwrap().into_boxed_slice(),
                    config.code_file,
                    config.execute_file,
                    shlex::split(&config.execute_args)
                        .unwrap().into_boxed_slice(),
                )) as Box<Compiler>
            },
            "interpreter" => {
                Box::new(Interpreter::new(
                    config.code_file,
                    config.execute_file,
                    shlex::split(&config.execute_args)
                        .unwrap().into_boxed_slice(),
                )) as Box<Compiler>
            }
            _ => panic!(),
        })
    }).collect()
}
