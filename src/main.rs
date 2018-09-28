#![recursion_limit = "128"]

extern crate cast;
extern crate clap;
extern crate either;
extern crate env_logger;
#[macro_use]
extern crate error_chain;
extern crate inflections;
#[macro_use]
extern crate log;
#[macro_use]
extern crate quote;
extern crate svd_parser as svd;
extern crate syn;

mod errors;
mod generate;
mod util;

use std::fs::File;
use std::process;
use std::io::Write;

use quote::Tokens;
use clap::{App, Arg};

use errors::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Target {
    CortexM,
    Msp430,
    RISCV,
    None,
}

impl Target {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "cortex-m" => Target::CortexM,
            "msp430" => Target::Msp430,
            "riscv" => Target::RISCV,
            "none" => Target::None,
            _ => bail!("unknown target {}", s),
        })
    }
}

fn run() -> Result<()> {
    use std::io::Read;

    let matches = App::new("svd2rust")
        .about("Generate a Rust API from SVD files")
        .arg(
            Arg::with_name("input")
                .help("Input SVD file")
                .short("i")
                .takes_value(true)
                .value_name("FILE"),
        )
        .arg(
            Arg::with_name("target")
                .long("target")
                .help("Target architecture")
                .takes_value(true)
                .value_name("ARCH"),
        )
        .arg(
            Arg::with_name("nightly_features")
                .long("nightly")
                .help("Enable features only available to nightly rustc")
        )
        .arg(
            Arg::with_name("log_level")
                .long("log")
                .short("l")
                .help(&format!(
                    "Choose which messages to log (overrides {})",
                    env_logger::DEFAULT_FILTER_ENV
                ))
                .takes_value(true)
                .possible_values(&["off", "error", "warn", "info", "debug", "trace"])
        )
        .version(concat!(
            env!("CARGO_PKG_VERSION"),
            include_str!(concat!(env!("OUT_DIR"), "/commit-info.txt"))
        ))
        .get_matches();

    setup_logging(&matches);

    let target = matches
        .value_of("target")
        .map(|s| Target::parse(s))
        .unwrap_or(Ok(Target::CortexM))?;

    let xml = &mut String::new();
    match matches.value_of("input") {
        Some(file) => {
            File::open(file)
                .chain_err(|| "couldn't open the SVD file")?
                .read_to_string(xml)
                .chain_err(|| "couldn't read the SVD file")?;
        }
        None => {
            let stdin = std::io::stdin();
            stdin
                .lock()
                .read_to_string(xml)
                .chain_err(|| "couldn't read from stdin")?;
        }
    }

    let device = svd::parse(xml);

    let nightly = matches.is_present("nightly_features");

    let mut device_x = String::new();
    let items = generate::device::render(&device, &target, nightly, &mut device_x)?;

    if target == Target::CortexM {
        writeln!(File::create("lib.rs").unwrap(), "{}", quote!(#(#items)*)).unwrap();
        writeln!(File::create("device.x").unwrap(), "{}", device_x).unwrap();
        writeln!(File::create("build.rs").unwrap(), "{}", build_rs()).unwrap();
    } else {
        println!(
            "{}",
            quote! {
                #(#items)*
            }
        );
    }

    Ok(())
}

fn setup_logging(matches: &clap::ArgMatches) {
    // * Log at info by default.
    // * Allow users the option of setting complex logging filters using
    //   env_logger's `RUST_LOG` environment variable.
    // * Override both of those if the logging level is set via the `--log`
    //   command line argument.
    let env = env_logger::Env::default()
        .filter_or(env_logger::DEFAULT_FILTER_ENV, "info");
    let mut builder = env_logger::Builder::from_env(env);
    builder.default_format_timestamp(false);

    let log_lvl_from_env =
        std::env::var_os(env_logger::DEFAULT_FILTER_ENV).is_some();

    if log_lvl_from_env {
        log::set_max_level(log::LevelFilter::Trace);
    } else {
        let level = match matches.value_of("log_level") {
            Some(lvl) => lvl.parse().unwrap(),
            None => log::LevelFilter::Info,
        };
        log::set_max_level(level);
        builder.filter_level(level);
    }

    builder.init();
}

fn main() {
    if let Err(ref e) = run() {
        error!("{}", e);

        for e in e.iter().skip(1) {
            error!("caused by: {}", e);
        }

        if let Some(backtrace) = e.backtrace() {
            error!("backtrace: {:?}", backtrace);
        } else {
            error!("note: run with `RUST_BACKTRACE=1` for a backtrace")
        }

        process::exit(1);
    }
}

fn build_rs() -> Tokens {
    quote! {
        use std::env;
        use std::fs::File;
        use std::io::Write;
        use std::path::PathBuf;

        fn main() {
            if env::var_os("CARGO_FEATURE_RT").is_some() {
                // Put the linker script somewhere the linker can find it
                let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
                File::create(out.join("device.x"))
                    .unwrap()
                    .write_all(include_bytes!("device.x"))
                    .unwrap();
                println!("cargo:rustc-link-search={}", out.display());

                println!("cargo:rerun-if-changed=device.x");
            }

            println!("cargo:rerun-if-changed=build.rs");
        }
    }
}
