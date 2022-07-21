use crate::store::StoreOptions;
use crate::warning;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use structopt::StructOpt;
use wasmer_compiler::ModuleEnvironment;
use wasmer_compiler::{ArtifactBuild, ArtifactCreate};
use wasmer_types::entity::PrimaryMap;
use wasmer_types::{
    CompileError, CpuFeature, MemoryIndex, MemoryStyle, TableIndex, TableStyle, Target, Triple,
};

#[derive(Debug, StructOpt)]
/// The options for the `wasmer compile` subcommand
pub struct Compile {
    /// Input file
    #[structopt(name = "FILE", parse(from_os_str))]
    path: PathBuf,

    /// Output file
    #[structopt(name = "OUTPUT PATH", short = "o", parse(from_os_str))]
    output: PathBuf,

    /// Compilation Target triple
    #[structopt(long = "target")]
    target_triple: Option<Triple>,

    #[structopt(flatten)]
    store: StoreOptions,

    #[structopt(short = "m", multiple = true, number_of_values = 1)]
    cpu_features: Vec<CpuFeature>,
}

impl Compile {
    /// Runs logic for the `compile` subcommand
    pub fn execute(&self) -> Result<()> {
        self.inner_execute()
            .context(format!("failed to compile `{}`", self.path.display()))
    }

    fn inner_execute(&self) -> Result<()> {
        let target = self
            .target_triple
            .as_ref()
            .map(|target_triple| {
                let mut features = self
                    .cpu_features
                    .clone()
                    .into_iter()
                    .fold(CpuFeature::set(), |a, b| a | b);
                // Cranelift requires SSE2, so we have this "hack" for now to facilitate
                // usage
                features |= CpuFeature::SSE2;
                Target::new(target_triple.clone(), features)
            })
            .unwrap_or_default();
        let (mut engine, compiler_type) = self.store.get_engine_for_target(target.clone())?;
        let output_filename = self
            .output
            .file_stem()
            .map(|osstr| osstr.to_string_lossy().to_string())
            .unwrap_or_default();
        // `.wasmu` is the default extension for all the triples. It
        // stands for “Wasm Universal”.
        let recommended_extension = "wasmu";
        match self.output.extension() {
            Some(ext) => {
                if ext != recommended_extension {
                    warning!("the output file has a wrong extension. We recommend using `{}.{}` for the chosen target", &output_filename, &recommended_extension)
                }
            }
            None => {
                warning!("the output file has no extension. We recommend using `{}.{}` for the chosen target", &output_filename, &recommended_extension)
            }
        }
        let tunables = self.store.get_tunables_for_target(&target)?;

        println!("Compiler: {}", compiler_type.to_string());
        println!("Target: {}", target.triple());

        // compile and save the artifact (without using module from api)
        let path: &Path = self.path.as_ref();
        let wasm_bytes = std::fs::read(path)?;
        let environ = ModuleEnvironment::new();
        let translation = environ.translate(&wasm_bytes).map_err(CompileError::Wasm)?;
        let module = translation.module;
        let memory_styles: PrimaryMap<MemoryIndex, MemoryStyle> = module
            .memories
            .values()
            .map(|memory_type| tunables.memory_style(memory_type))
            .collect();
        let table_styles: PrimaryMap<TableIndex, TableStyle> = module
            .tables
            .values()
            .map(|table_type| tunables.table_style(table_type))
            .collect();
        let artifact = ArtifactBuild::new(
            &mut engine,
            &wasm_bytes,
            &target,
            memory_styles,
            table_styles,
        )?;
        artifact.serialize_to_file(self.output.as_ref())?;
        eprintln!(
            "✔ File compiled successfully to `{}`.",
            self.output.display(),
        );

        Ok(())
    }
}
