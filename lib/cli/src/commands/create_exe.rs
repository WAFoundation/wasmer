//! Create a standalone native executable for a given Wasm file.

use super::ObjectFormat;
use crate::store::CompilerOptions;
use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::Command;
use structopt::StructOpt;
use wasmer::*;
use wasmer_object::{emit_serialized, get_object_for_target};

const WASMER_MAIN_C_SOURCE: &[u8] = include_bytes!("wasmer_create_exe_main.c");
#[cfg(feature = "static-artifact-create")]
const WASMER_STATIC_MAIN_C_SOURCE: &[u8] = include_bytes!("wasmer_static_create_exe_main.c");

#[derive(Debug, StructOpt)]
/// The options for the `wasmer create-exe` subcommand
pub struct CreateExe {
    /// Input file
    #[structopt(name = "FILE", parse(from_os_str))]
    path: PathBuf,

    /// Output file
    #[structopt(name = "OUTPUT PATH", short = "o", parse(from_os_str))]
    output: PathBuf,

    /// Compilation Target triple
    #[structopt(long = "target")]
    target_triple: Option<Triple>,

    /// Cross-compilation library path.
    #[structopt(long = "cross-compilation-library-path", requires = "target_triple")]
    library_path: Option<PathBuf>,

    /// Object format options
    ///
    /// This flag accepts two options: `symbols` or `serialized`.
    /// - (default) `symbols` creates an
    /// executable where all functions and metadata of the module are regular object symbols
    /// - `serialized` creates an executable where the module is zero-copy serialized as raw data
    #[structopt(name = "OBJECT_FORMAT", long = "object-format", verbatim_doc_comment)]
    object_format: Option<ObjectFormat>,

    /// Header file for object input
    ///
    /// If given, the input `PATH` is assumed to be an object created with `wasmer create-obj` and
    /// this is its accompanying header file.
    #[structopt(name = "HEADER", long = "header", verbatim_doc_comment)]
    header: Option<PathBuf>,

    #[structopt(short = "m", multiple = true, number_of_values = 1)]
    cpu_features: Vec<CpuFeature>,

    /// Additional libraries to link against.
    /// This is useful for fixing linker errors that may occur on some systems.
    #[structopt(short = "l", multiple = true, number_of_values = 1)]
    libraries: Vec<String>,

    #[structopt(flatten)]
    compiler: CompilerOptions,
}

impl CreateExe {
    /// Runs logic for the `compile` subcommand
    pub fn execute(&self) -> Result<()> {
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
                if target_triple.architecture == Architecture::X86_64 {
                    features |= CpuFeature::SSE2;
                }
                Target::new(target_triple.clone(), features)
            })
            .unwrap_or_default();

        let working_dir = tempfile::tempdir()?;
        let starting_cd = env::current_dir()?;
        let output_path = starting_cd.join(&self.output);
        env::set_current_dir(&working_dir)?;
        let mut library_path: Option<PathBuf> = None;

        if *target.triple() != Triple::host() {
            library_path = if let v @ Some(_) = self.library_path.clone() {
                v
            } else {
                let latest_release = http_fetch::get_latest_release()?;
                let tarball =
                    http_fetch::download_release(latest_release, Some(target.triple().clone()))?;
                let files = http_fetch::untar(tarball)?;
                let filename = files.into_iter().find(|f| f.contains("lib/libwasmer.a")).ok_or_else(|| {
                    anyhow!("Could not find libwasmer for {} target in the fetched release from Github: you can download it manually and specify its path with the --cross-compilation-library-path LIBRARY_PATH flag.", target.triple())})?;
                Some(filename.into())
            };
        }
        let (store, compiler_type) = self.compiler.get_store_for_target(target.clone())?;
        let object_format = self.object_format.unwrap_or(ObjectFormat::Symbols);
        let library_path = library_path.map(|v| working_dir.path().join(&v));

        println!("Compiler: {}", compiler_type.to_string());
        println!("Target: {}", target.triple());
        if let Some(p) = library_path.as_ref() {
            println!("Library Path: {}", p.display());
        }
        println!("Format: {:?}", object_format);

        #[cfg(not(windows))]
        let wasm_object_path = PathBuf::from("wasm.o");
        #[cfg(windows)]
        let wasm_object_path = PathBuf::from("wasm.obj");

        let wasm_module_path = starting_cd.join(&self.path);

        if let Some(header_path) = self.header.as_ref() {
            let header_path = starting_cd.join(&header_path);
            std::fs::copy(&header_path, Path::new("static_defs.h"))
                .context("Could not access given header file")?;
            link(
                output_path,
                wasm_module_path,
                std::path::Path::new("static_defs.h").into(),
                library_path,
                self.target_triple.clone(),
            )?;
        } else {
            match object_format {
                ObjectFormat::Serialized => {
                    let module = Module::from_file(&store, &wasm_module_path)
                        .context("failed to compile Wasm")?;
                    let bytes = module.serialize()?;
                    let mut obj = get_object_for_target(target.triple())?;
                    emit_serialized(&mut obj, &bytes, target.triple())?;
                    let mut writer = BufWriter::new(File::create(&wasm_object_path)?);
                    obj.write_stream(&mut writer)
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
                    writer.flush()?;
                    drop(writer);

                    self.compile_c(wasm_object_path, output_path)?;
                }
                #[cfg(not(feature = "static-artifact-create"))]
                ObjectFormat::Symbols => {
                    return Err(anyhow!("This version of wasmer-cli hasn't been compiled with static artifact support. You need to enable the `static-artifact-create` feature during compilation."));
                }
                #[cfg(feature = "static-artifact-create")]
                ObjectFormat::Symbols => {
                    let engine = store.engine();
                    let engine_inner = engine.inner();
                    let compiler = engine_inner.compiler()?;
                    let features = engine_inner.features();
                    let tunables = store.tunables();
                    let data: Vec<u8> = fs::read(wasm_module_path)?;
                    let prefixer: Option<Box<dyn Fn(&[u8]) -> String + Send>> = None;
                    let (module_info, obj, metadata_length, symbol_registry) =
                        Artifact::generate_object(
                            compiler, &data, prefixer, &target, tunables, features,
                        )?;

                    let header_file_src = crate::c_gen::staticlib_header::generate_header_file(
                        &module_info,
                        &*symbol_registry,
                        metadata_length,
                    );
                    /* Write object file with functions */
                    let object_file_path: std::path::PathBuf =
                        std::path::Path::new("functions.o").into();
                    let mut writer = BufWriter::new(File::create(&object_file_path)?);
                    obj.write_stream(&mut writer)
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
                    writer.flush()?;
                    /* Write down header file that includes pointer arrays and the deserialize function
                     * */
                    let mut writer = BufWriter::new(File::create("static_defs.h")?);
                    writer.write_all(header_file_src.as_bytes())?;
                    writer.flush()?;
                    link(
                        output_path,
                        object_file_path,
                        std::path::Path::new("static_defs.h").into(),
                        library_path,
                        self.target_triple.clone(),
                    )?;
                }
            }
        }
        eprintln!(
            "✔ Native executable compiled successfully to `{}`.",
            self.output.display(),
        );

        Ok(())
    }

    fn compile_c(&self, wasm_object_path: PathBuf, output_path: PathBuf) -> anyhow::Result<()> {
        // write C src to disk
        let c_src_path = Path::new("wasmer_main.c");
        #[cfg(not(windows))]
        let c_src_obj = PathBuf::from("wasmer_main.o");
        #[cfg(windows)]
        let c_src_obj = PathBuf::from("wasmer_main.obj");

        {
            let mut c_src_file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&c_src_path)
                .context("Failed to open C source code file")?;
            c_src_file.write_all(WASMER_MAIN_C_SOURCE)?;
        }
        run_c_compile(c_src_path, &c_src_obj, self.target_triple.clone())
            .context("Failed to compile C source code")?;
        LinkCode {
            object_paths: vec![c_src_obj, wasm_object_path],
            output_path,
            additional_libraries: self.libraries.clone(),
            target: self.target_triple.clone(),
            ..Default::default()
        }
        .run()
        .context("Failed to link objects together")?;

        Ok(())
    }
}

#[cfg(feature = "static-artifact-create")]
fn link(
    output_path: PathBuf,
    object_path: PathBuf,
    mut header_code_path: PathBuf,
    library_path: Option<PathBuf>,
    target_triple: Option<wasmer::Triple>,
) -> anyhow::Result<()> {
    let linkcode = LinkCode {
        object_paths: vec![object_path.clone(), "main_obj.obj".into()],
        output_path: output_path.clone(),
        ..Default::default()
    };
    let c_src_path = Path::new("wasmer_main.c");
    let mut libwasmer_path = if let Some(p) = std::dbg!(library_path) {
        p
    } else {
        get_libwasmer_path()?
    }
    .canonicalize()
    .context("Failed to find libwasmer")?;
    println!("Using libwasmer: {}", libwasmer_path.display());
    let lib_filename = libwasmer_path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    libwasmer_path.pop();
    {
        let mut c_src_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&c_src_path)
            .context("Failed to open C source code file")?;
        c_src_file.write_all(WASMER_STATIC_MAIN_C_SOURCE)?;
    }

    if !header_code_path.is_dir() {
        header_code_path.pop();
    }
    if header_code_path.display().to_string().is_empty() {
        header_code_path = std::env::current_dir()?;
    }

    /* Compile main function */
    let compilation = if let Some(triple) = target_triple.clone() {
        let mut include_dir = libwasmer_path.clone();
        include_dir.pop();
        include_dir.push("include");
        let zig_triple = triple_to_zig_triple(triple);
        std::dbg!(&libwasmer_path);
        std::dbg!(&include_dir);
        std::dbg!(&lib_filename);
        std::dbg!(&header_code_path);
        std::dbg!(&c_src_path);
        std::dbg!(&zig_triple);
        // zig cc -target x86_64-macos.12.3.1...12.3.1-none -L/Users/manos/Downloads/wasmer-darwin-amd64/lib -l:libwasmer.a -I`pwd`/package/include qjs.obj lib/cli/src/commands/wasmer_static_create_exe_main.c
        // zig build-exe -target x86_64-macos-none -L/Users/manos/Downloads/wasmer-darwin-amd64/lib -l:libwasmer.a -I`pwd`/package/include qjs.obj lib/cli/src/commands/wasmer_static_create_exe_main.c
        // zig cc -target x86_64-macos-none -L`pwd`/lib -l:libwasmer.a -I`pwd`/include -I`pwd` qjs.o ../lib/cli/src/commands/wasmer_static_create_exe_main.c
        Command::new("zig")
            .arg("cc")
            .arg("-target")
            .arg(&zig_triple)
            .arg(&format!("-L{}", libwasmer_path.display()))
            .arg(&format!("-l:{}", lib_filename))
            .arg(&format!("-I{}", include_dir.display()))
            .arg(&format!("-I{}", header_code_path.display()))
            .arg(&object_path)
            .arg(&c_src_path)
            .arg("-o")
            .arg(&output_path)
            .output()?
    } else {
        Command::new("cc")
            .arg("-c")
            .arg(&c_src_path)
            .arg(if linkcode.optimization_flag.is_empty() {
                "-O2"
            } else {
                linkcode.optimization_flag.as_str()
            })
            .arg(&format!("-L{}", libwasmer_path.display()))
            .arg(&format!("-I{}", get_wasmer_include_directory()?.display()))
            .arg(&format!("-l:{}", lib_filename))
            //.arg("-lwasmer")
            // Add libraries required per platform.
            // We need userenv, sockets (Ws2_32), advapi32 for some system calls and bcrypt for random numbers.
            //#[cfg(windows)]
            //    .arg("-luserenv")
            //    .arg("-lWs2_32")
            //    .arg("-ladvapi32")
            //    .arg("-lbcrypt")
            // On unix we need dlopen-related symbols, libmath for a few things, and pthreads.
            //#[cfg(not(windows))]
            .arg("-ldl")
            .arg("-lm")
            .arg("-pthread")
            .arg(&format!("-I{}", header_code_path.display()))
            .arg("-v")
            .arg("-o")
            .arg("main_obj.obj")
            .output()?
    };
    if !compilation.status.success() {
        return Err(anyhow::anyhow!(String::from_utf8_lossy(
            &compilation.stderr
        )
        .to_string()));
    }
    if target_triple.is_none() {
        linkcode.run().context("Failed to link objects together")?;
    }
    Ok(())
}

fn triple_to_zig_triple(target_triple: Triple) -> String {
    let arch = match target_triple.architecture {
        wasmer_types::Architecture::X86_64 => "x86_64".into(),
        wasmer_types::Architecture::Aarch64(wasmer_types::Aarch64Architecture::Aarch64) => {
            "aarch64".into()
        }
        v => v.to_string(),
    };
    let os = match target_triple.operating_system {
        wasmer_types::OperatingSystem::Linux => "linux".into(),
        wasmer_types::OperatingSystem::Darwin => "macos".into(),
        wasmer_types::OperatingSystem::Windows => "windows".into(),
        v => v.to_string(),
    };
    let env = match target_triple.environment {
        wasmer_types::Environment::Musl => "musl",
        _ => "none",
    };
    format!("{}-{}-{}", arch, os, env)
}

fn get_wasmer_dir() -> anyhow::Result<PathBuf> {
    Ok(PathBuf::from(
        env::var("WASMER_DIR")
            .or_else(|e| {
                option_env!("WASMER_INSTALL_PREFIX")
                    .map(str::to_string)
                    .ok_or(e)
            })
            .context("Trying to read env var `WASMER_DIR`")?,
    ))
}

fn get_wasmer_include_directory() -> anyhow::Result<PathBuf> {
    let mut path = get_wasmer_dir()?;
    path.push("include");
    Ok(path)
}

/// path to the static libwasmer
fn get_libwasmer_path() -> anyhow::Result<PathBuf> {
    let mut path = get_wasmer_dir()?;
    path.push("lib");

    // TODO: prefer headless Wasmer if/when it's a separate library.
    #[cfg(not(windows))]
    path.push("libwasmer.a");
    #[cfg(windows)]
    path.push("wasmer.lib");

    Ok(path)
}

/// Compile the C code.
fn run_c_compile(
    path_to_c_src: &Path,
    output_name: &Path,
    target: Option<Triple>,
) -> anyhow::Result<()> {
    #[cfg(not(windows))]
    let c_compiler = "cc";
    // We must use a C++ compiler on Windows because wasm.h uses `static_assert`
    // which isn't available in `clang` on Windows.
    #[cfg(windows)]
    let c_compiler = "clang++";

    let mut command = Command::new(c_compiler);
    let command = command
        .arg("-O2")
        .arg("-c")
        .arg(path_to_c_src)
        .arg("-I")
        .arg(get_wasmer_include_directory()?);

    let command = if let Some(target) = target {
        command.arg("-target").arg(format!("{}", target))
    } else {
        command
    };

    let output = command.arg("-o").arg(output_name).output()?;

    if !output.status.success() {
        bail!(
            "C code compile failed with: stdout: {}\n\nstderr: {}",
            std::str::from_utf8(&output.stdout)
                .expect("stdout is not utf8! need to handle arbitrary bytes"),
            std::str::from_utf8(&output.stderr)
                .expect("stderr is not utf8! need to handle arbitrary bytes")
        );
    }
    Ok(())
}

/// Data used to run a linking command for generated artifacts.
#[derive(Debug)]
struct LinkCode {
    /// Path to the linker used to run the linking command.
    linker_path: PathBuf,
    /// String used as an optimization flag.
    optimization_flag: String,
    /// Paths of objects to link.
    object_paths: Vec<PathBuf>,
    /// Additional libraries to link against.
    additional_libraries: Vec<String>,
    /// Path to the output target.
    output_path: PathBuf,
    /// Path to the dir containing the static libwasmer library.
    libwasmer_path: PathBuf,
    /// The target to link the executable for.
    target: Option<Triple>,
}

impl Default for LinkCode {
    fn default() -> Self {
        #[cfg(not(windows))]
        let linker = "cc";
        #[cfg(windows)]
        let linker = "clang";
        Self {
            linker_path: PathBuf::from(linker),
            optimization_flag: String::from("-O2"),
            object_paths: vec![],
            additional_libraries: vec![],
            output_path: PathBuf::from("a.out"),
            libwasmer_path: get_libwasmer_path().unwrap(),
            target: None,
        }
    }
}

impl LinkCode {
    fn run(&self) -> anyhow::Result<()> {
        let libwasmer_path = self
            .libwasmer_path
            .canonicalize()
            .context("Failed to find libwasmer")?;
        println!(
            "Using path `{}` as libwasmer path.",
            libwasmer_path.display()
        );
        let mut command = Command::new(&self.linker_path);
        let command = command
            .arg(&self.optimization_flag)
            .args(
                self.object_paths
                    .iter()
                    .map(|path| path.canonicalize().unwrap()),
            )
            .arg(&libwasmer_path);
        let command = if let Some(target) = &self.target {
            command.arg("-target").arg(format!("{}", target))
        } else {
            command
        };
        // Add libraries required per platform.
        // We need userenv, sockets (Ws2_32), advapi32 for some system calls and bcrypt for random numbers.
        #[cfg(windows)]
        let command = command
            .arg("-luserenv")
            .arg("-lWs2_32")
            .arg("-ladvapi32")
            .arg("-lbcrypt");
        // On unix we need dlopen-related symbols, libmath for a few things, and pthreads.
        #[cfg(not(windows))]
        let command = command.arg("-ldl").arg("-lm").arg("-pthread");
        let link_against_extra_libs = self
            .additional_libraries
            .iter()
            .map(|lib| format!("-l{}", lib));
        let command = command.args(link_against_extra_libs);
        let output = command.arg("-o").arg(&self.output_path).output()?;

        if !output.status.success() {
            bail!(
                "linking failed with: stdout: {}\n\nstderr: {}",
                std::str::from_utf8(&output.stdout)
                    .expect("stdout is not utf8! need to handle arbitrary bytes"),
                std::str::from_utf8(&output.stderr)
                    .expect("stderr is not utf8! need to handle arbitrary bytes")
            );
        }
        Ok(())
    }
}

#[cfg(feature = "http")]
mod http_fetch {
    use anyhow::{anyhow, Context, Result};
    use http_req::{
        request::Request,
        response::{Response, StatusCode},
        uri::Uri,
    };
    use std::convert::TryFrom;

    pub fn get_latest_release() -> Result<serde_json::Value> {
        let mut writer = Vec::new();
        let uri = Uri::try_from("https://api.github.com/repos/wasmerio/wasmer/releases").unwrap();

        let response = Request::new(&uri)
            .header("User-Agent", "wasmer.io self update")
            .header("Accept", "application/vnd.github.v3+json")
            .timeout(Some(std::time::Duration::new(30, 0)))
            .send(&mut writer)
            .map_err(anyhow::Error::new)
            .context("Could not lookup wasmer repository on Github.")?;

        assert_eq!(response.status_code(), StatusCode::new(200));

        let v: std::result::Result<serde_json::Value, _> = serde_json::from_reader(&*writer);
        let mut response = v.map_err(anyhow::Error::new)?;
        eprintln!("response: {:?}", &response);
        if let Some(releases) = response.as_array_mut() {
            releases.retain(|r| {
                r["tag_name"].is_string() && !r["tag_name"].as_str().unwrap().is_empty()
            });
            releases.sort_by_cached_key(|r| r["tag_name"].as_str().unwrap_or_default().to_string());
            if let Some(latest) = releases.pop() {
                return Ok(latest);
            }
        }

        Err(anyhow!(
            "Could not get Github API response, falling back to downloading latest version."
        ))
    }

    pub fn download_release(
        mut latest: serde_json::Value,
        target_triple: Option<wasmer::Triple>,
    ) -> Result<String> {
        eprintln!("latest: {:?}", &latest);
        let target_triple = target_triple.unwrap_or(wasmer::Triple::host());
        if let Some(assets) = latest["assets"].as_array_mut() {
            assets.retain(|a| {
                if let Some(name) = a["name"].as_str() {
                    match target_triple.architecture {
                        wasmer_types::Architecture::X86_64 => {
                            name.contains("x86_64") || name.contains("amd64")
                        }
                        wasmer_types::Architecture::Aarch64(
                            wasmer_types::Aarch64Architecture::Aarch64,
                        ) => name.contains("arm64") || name.contains("aarch64"),
                        _ => false,
                    }
                } else {
                    false
                }
            });
            assets.retain(|a| {
                if let Some(name) = a["name"].as_str() {
                    match target_triple.vendor {
                        wasmer_types::Vendor::Apple => !name.contains("windows"),
                        wasmer_types::Vendor::Pc => !name.contains("apple"),
                        _ => true,
                    }
                } else {
                    false
                }
            });
            assets.retain(|a| {
                if let Some(name) = a["name"].as_str() {
                    match target_triple.operating_system {
                        wasmer_types::OperatingSystem::Darwin => {
                            name.contains("darwin") || name.contains("macos")
                        }
                        wasmer_types::OperatingSystem::Windows => name.contains("windows"),
                        wasmer_types::OperatingSystem::Linux => name.contains("linux"),
                        _ => false,
                    }
                } else {
                    false
                }
            });
            assets.retain(|a| {
                if let Some(name) = a["name"].as_str() {
                    match target_triple.environment {
                        wasmer_types::Environment::Musl => name.contains("musl"),
                        _ => !name.contains("musl"),
                    }
                } else {
                    false
                }
            });
            println!("assets retained: {:?}", &assets);
            if assets.len() == 1 {
                let browser_download_url =
                    if let Some(url) = assets[0]["browser_download_url"].as_str() {
                        url.to_string()
                    } else {
                        return Err(anyhow!(
                            "Could not get download url from Github API response."
                        ));
                    };
                let filename = browser_download_url
                    .split("/")
                    .last()
                    .unwrap_or("output")
                    .to_string();
                let mut file = std::fs::File::create(&filename)?;
                println!("Downloading {} to {}", browser_download_url, &filename);
                let download_thread: std::thread::JoinHandle<Result<Response, anyhow::Error>> =
                    std::thread::spawn(move || {
                        let uri = Uri::try_from(browser_download_url.as_str())?;
                        let mut response = Request::new(&uri)
                            .header("User-Agent", "wasmer")
                            .send(&mut file)
                            .map_err(anyhow::Error::new)
                            .context("Could not lookup wasmer artifact on Github.")?;
                        if response.status_code() == StatusCode::new(302) {
                            let redirect_uri =
                                Uri::try_from(response.headers().get("Location").unwrap().as_str())
                                    .unwrap();
                            response = Request::new(&redirect_uri)
                                .header("User-Agent", "wasmer")
                                .send(&mut file)
                                .map_err(anyhow::Error::new)
                                .context("Could not lookup wasmer artifact on Github.")?;
                        }
                        Ok(response)
                    });
                let _sleep_dur = std::time::Duration::from_millis(1);
                let _sleep_ctr = std::time::Duration::from_millis(0);

                println!();

                /*
                while !download_thread.is_finished() {
                    sleep_ctr = sleep_ctr.checked_add(sleep_dur).unwrap_or(sleep_ctr);
                    match std::fs::metadata(&filename) {
                        Ok(v) => {
                            print!("\r{} bytes", v.len());
                            std::io::stdout().flush();
                        }
                        Err(err) => {
                            println!("Could not read `{}` file metadata: {}", &filename, err);
                        }
                    }
                    std::thread::sleep(sleep_dur);
                }
                println!();
                */
                let _response = download_thread
                    .join()
                    .expect("Could not join downloading thread");
                //file.write_all(&writer)?;
                //println!("downloaded {} bytes to {}", writer.len(), filename);
                return Ok(filename);
            }
        }
        Err(anyhow!("Could not get latest release artifact."))
    }

    pub fn untar(tarball: String) -> Result<Vec<String>> {
        let files = std::process::Command::new("tar")
            .arg("-tf")
            .arg(&tarball)
            .output()
            .expect("failed to execute process")
            .stdout;
        let files_s = String::from_utf8(files)?;
        std::dbg!(&files_s); //debug
        let files = files_s
            .lines()
            .filter(|p| !p.ends_with('/'))
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        std::dbg!(&files); //debug
        let _output = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&tarball)
            .output()
            .expect("failed to execute process");
        Ok(files)
    }
}
