// Copyright 2018-2023 the Deno authors. All rights reserved. MIT license.

use std::env;
use std::path::Path;
use std::path::PathBuf;

use deno_core::include_js_files_dir;
use deno_core::snapshot_util::*;
use deno_core::Extension;
use deno_core::ExtensionFileSource;
use deno_runtime::deno_cache::SqliteBackedCache;
use deno_runtime::permissions::PermissionsContainer;
use deno_runtime::*;

mod ts {
  use super::*;
  use crate::deno_webgpu_get_declaration;
  use deno_core::error::custom_error;
  use deno_core::error::AnyError;
  use deno_core::include_js_files_dir;
  use deno_core::op;
  use deno_core::OpState;
  use deno_runtime::deno_node::SUPPORTED_BUILTIN_NODE_MODULES;
  use regex::Regex;
  use serde::Deserialize;
  use serde_json::json;
  use serde_json::Value;
  use std::collections::HashMap;
  use std::path::Path;
  use std::path::PathBuf;

  #[derive(Debug, Deserialize)]
  struct LoadArgs {
    /// The fully qualified specifier that should be loaded.
    specifier: String,
  }

  pub fn create_compiler_snapshot(snapshot_path: PathBuf, cwd: &Path) {
    // libs that are being provided by op crates.
    let mut op_crate_libs = HashMap::new();
    op_crate_libs.insert("deno.cache", deno_cache::get_declaration());
    op_crate_libs.insert("deno.console", deno_console::get_declaration());
    op_crate_libs.insert("deno.url", deno_url::get_declaration());
    op_crate_libs.insert("deno.web", deno_web::get_declaration());
    op_crate_libs.insert("deno.fetch", deno_fetch::get_declaration());
    op_crate_libs.insert("deno.webgpu", deno_webgpu_get_declaration());
    op_crate_libs.insert("deno.websocket", deno_websocket::get_declaration());
    op_crate_libs.insert("deno.webstorage", deno_webstorage::get_declaration());
    op_crate_libs.insert("deno.crypto", deno_crypto::get_declaration());
    op_crate_libs.insert(
      "deno.broadcast_channel",
      deno_broadcast_channel::get_declaration(),
    );
    op_crate_libs.insert("deno.net", deno_net::get_declaration());

    // ensure we invalidate the build properly.
    for (_, path) in op_crate_libs.iter() {
      println!("cargo:rerun-if-changed={}", path.display());
    }

    // libs that should be loaded into the isolate before snapshotting.
    let libs = vec![
      // Deno custom type libraries
      "deno.window",
      "deno.worker",
      "deno.shared_globals",
      "deno.ns",
      "deno.unstable",
      // Deno built-in type libraries
      "es5",
      "es2015.collection",
      "es2015.core",
      "es2015",
      "es2015.generator",
      "es2015.iterable",
      "es2015.promise",
      "es2015.proxy",
      "es2015.reflect",
      "es2015.symbol",
      "es2015.symbol.wellknown",
      "es2016.array.include",
      "es2016",
      "es2017",
      "es2017.intl",
      "es2017.object",
      "es2017.sharedmemory",
      "es2017.string",
      "es2017.typedarrays",
      "es2018.asyncgenerator",
      "es2018.asynciterable",
      "es2018",
      "es2018.intl",
      "es2018.promise",
      "es2018.regexp",
      "es2019.array",
      "es2019",
      "es2019.intl",
      "es2019.object",
      "es2019.string",
      "es2019.symbol",
      "es2020.bigint",
      "es2020",
      "es2020.date",
      "es2020.intl",
      "es2020.number",
      "es2020.promise",
      "es2020.sharedmemory",
      "es2020.string",
      "es2020.symbol.wellknown",
      "es2021",
      "es2021.intl",
      "es2021.promise",
      "es2021.string",
      "es2021.weakref",
      "es2022",
      "es2022.array",
      "es2022.error",
      "es2022.intl",
      "es2022.object",
      "es2022.sharedmemory",
      "es2022.string",
      "esnext",
      "esnext.array",
      "esnext.intl",
    ];

    let path_dts = cwd.join("tsc/dts");
    // ensure we invalidate the build properly.
    for name in libs.iter() {
      println!(
        "cargo:rerun-if-changed={}",
        path_dts.join(format!("lib.{name}.d.ts")).display()
      );
    }
    println!(
      "cargo:rerun-if-changed={}",
      cwd.join("tsc").join("00_typescript.js").display()
    );
    println!(
      "cargo:rerun-if-changed={}",
      cwd.join("tsc").join("99_main_compiler.js").display()
    );
    println!(
      "cargo:rerun-if-changed={}",
      cwd.join("js").join("40_testing.js").display()
    );

    // create a copy of the vector that includes any op crate libs to be passed
    // to the JavaScript compiler to build into the snapshot
    let mut build_libs = libs.clone();
    for (op_lib, _) in op_crate_libs.iter() {
      build_libs.push(op_lib.to_owned());
    }

    // used in the tests to verify that after snapshotting it has the same number
    // of lib files loaded and hasn't included any ones lazily loaded from Rust
    std::fs::write(
      PathBuf::from(env::var_os("OUT_DIR").unwrap())
        .join("lib_file_names.json"),
      serde_json::to_string(&build_libs).unwrap(),
    )
    .unwrap();

    #[op]
    fn op_build_info(state: &mut OpState) -> Value {
      let build_specifier = "asset:///bootstrap.ts";

      let node_built_in_module_names = SUPPORTED_BUILTIN_NODE_MODULES
        .iter()
        .map(|s| s.name)
        .collect::<Vec<&str>>();
      let build_libs = state.borrow::<Vec<&str>>();
      json!({
        "buildSpecifier": build_specifier,
        "libs": build_libs,
        "nodeBuiltInModuleNames": node_built_in_module_names,
      })
    }

    #[op]
    fn op_cwd() -> String {
      "cache:///".into()
    }

    #[op]
    fn op_exists() -> bool {
      false
    }

    #[op]
    fn op_is_node_file() -> bool {
      false
    }

    #[op]
    fn op_script_version(
      _state: &mut OpState,
      _args: Value,
    ) -> Result<Option<String>, AnyError> {
      Ok(Some("1".to_string()))
    }

    #[op]
    // using the same op that is used in `tsc.rs` for loading modules and reading
    // files, but a slightly different implementation at build time.
    fn op_load(state: &mut OpState, args: LoadArgs) -> Result<Value, AnyError> {
      let op_crate_libs = state.borrow::<HashMap<&str, PathBuf>>();
      let path_dts = state.borrow::<PathBuf>();
      let re_asset =
        Regex::new(r"asset:/{3}lib\.(\S+)\.d\.ts").expect("bad regex");
      let build_specifier = "asset:///bootstrap.ts";

      // we need a basic file to send to tsc to warm it up.
      if args.specifier == build_specifier {
        Ok(json!({
          "data": r#"Deno.writeTextFile("hello.txt", "hello deno!");"#,
          "version": "1",
          // this corresponds to `ts.ScriptKind.TypeScript`
          "scriptKind": 3
        }))
        // specifiers come across as `asset:///lib.{lib_name}.d.ts` and we need to
        // parse out just the name so we can lookup the asset.
      } else if let Some(caps) = re_asset.captures(&args.specifier) {
        if let Some(lib) = caps.get(1).map(|m| m.as_str()) {
          // if it comes from an op crate, we were supplied with the path to the
          // file.
          let path = if let Some(op_crate_lib) = op_crate_libs.get(lib) {
            PathBuf::from(op_crate_lib).canonicalize()?
            // otherwise we are will generate the path ourself
          } else {
            path_dts.join(format!("lib.{lib}.d.ts"))
          };
          let data = std::fs::read_to_string(path)?;
          Ok(json!({
            "data": data,
            "version": "1",
            // this corresponds to `ts.ScriptKind.TypeScript`
            "scriptKind": 3
          }))
        } else {
          Err(custom_error(
            "InvalidSpecifier",
            format!("An invalid specifier was requested: {}", args.specifier),
          ))
        }
      } else {
        Err(custom_error(
          "InvalidSpecifier",
          format!("An invalid specifier was requested: {}", args.specifier),
        ))
      }
    }

    let tsc_extension = Extension::builder("deno_tsc")
      .ops(vec![
        op_build_info::decl(),
        op_cwd::decl(),
        op_exists::decl(),
        op_is_node_file::decl(),
        op_load::decl(),
        op_script_version::decl(),
      ])
      .js(include_js_files_dir! {
        dir "tsc",
        "00_typescript.js",
        "99_main_compiler.js",
      })
      .state(move |state| {
        state.put(op_crate_libs.clone());
        state.put(build_libs.clone());
        state.put(path_dts.clone());

        Ok(())
      })
      .build();

    create_snapshot(CreateSnapshotOptions {
      cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
      snapshot_path,
      startup_snapshot: None,
      extensions: vec![],
      extensions_with_js: vec![tsc_extension],
      compression_cb: Some(Box::new(|vec, snapshot_slice| {
        vec.extend_from_slice(
          &zstd::bulk::compress(snapshot_slice, 22)
            .expect("snapshot compression failed"),
        );
      })),
      snapshot_module_load_cb: None,
    });
  }

  pub(crate) fn version() -> String {
    let file_text = std::fs::read_to_string("tsc/00_typescript.js").unwrap();
    let mut version = String::new();
    for line in file_text.lines() {
      let major_minor_text = "ts.versionMajorMinor = \"";
      let version_text = "ts.version = \"\".concat(ts.versionMajorMinor, \"";
      if version.is_empty() {
        if let Some(index) = line.find(major_minor_text) {
          let remaining_line = &line[index + major_minor_text.len()..];
          version
            .push_str(&remaining_line[..remaining_line.find('"').unwrap()]);
        }
      } else if let Some(index) = line.find(version_text) {
        let remaining_line = &line[index + version_text.len()..];
        version.push_str(&remaining_line[..remaining_line.find('"').unwrap()]);
        return version;
      }
    }
    panic!("Could not find ts version.")
  }
}

fn create_cli_snapshot(snapshot_path: PathBuf) {
  let extensions: Vec<Extension> = vec![
    deno_webidl::init(),
    deno_console::init(),
    deno_url::init(),
    deno_tls::init(),
    deno_web::init::<PermissionsContainer>(
      deno_web::BlobStore::default(),
      Default::default(),
    ),
    deno_fetch::init::<PermissionsContainer>(Default::default()),
    deno_cache::init::<SqliteBackedCache>(None),
    deno_websocket::init::<PermissionsContainer>("".to_owned(), None, None),
    deno_webstorage::init(None),
    deno_crypto::init(None),
    deno_webgpu::init(false),
    deno_broadcast_channel::init(
      deno_broadcast_channel::InMemoryBroadcastChannel::default(),
      false, // No --unstable.
    ),
    deno_node::init::<PermissionsContainer>(None), // No --unstable.
    deno_ffi::init::<PermissionsContainer>(false),
    deno_net::init::<PermissionsContainer>(
      None, false, // No --unstable.
      None,
    ),
    deno_napi::init::<PermissionsContainer>(false),
    deno_http::init(),
    deno_flash::init::<PermissionsContainer>(false), // No --unstable
  ];

  let mut esm_files = include_js_files_dir!(
    dir "js",
    "40_testing.js",
  );
  esm_files.push(ExtensionFileSource {
    specifier: "runtime/js/99_main.js".to_string(),
    code: deno_runtime::js::SOURCE_CODE_FOR_99_MAIN_JS,
  });
  let extensions_with_js =
    vec![Extension::builder("cli").esm(esm_files).build()];

  create_snapshot(CreateSnapshotOptions {
    cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
    snapshot_path,
    startup_snapshot: Some(deno_runtime::js::deno_isolate_init()),
    extensions,
    extensions_with_js,
    compression_cb: Some(Box::new(|vec, snapshot_slice| {
      lzzzz::lz4_hc::compress_to_vec(
        snapshot_slice,
        vec,
        lzzzz::lz4_hc::CLEVEL_MAX,
      )
      .expect("snapshot compression failed");
    })),
    snapshot_module_load_cb: None,
  })
}

fn git_commit_hash() -> String {
  if let Ok(output) = std::process::Command::new("git")
    .arg("rev-list")
    .arg("-1")
    .arg("HEAD")
    .output()
  {
    if output.status.success() {
      std::str::from_utf8(&output.stdout[..40])
        .unwrap()
        .to_string()
    } else {
      // When not in git repository
      // (e.g. when the user install by `cargo install deno`)
      "UNKNOWN".to_string()
    }
  } else {
    // When there is no git command for some reason
    "UNKNOWN".to_string()
  }
}

fn main() {
  // Skip building from docs.rs.
  if env::var_os("DOCS_RS").is_some() {
    return;
  }

  // Host snapshots won't work when cross compiling.
  let target = env::var("TARGET").unwrap();
  let host = env::var("HOST").unwrap();
  if target != host {
    panic!("Cross compiling with snapshot is not supported.");
  }

  let symbols_path = std::path::Path::new("napi").join(
    format!("generated_symbol_exports_list_{}.def", env::consts::OS).as_str(),
  )
  .canonicalize()
  .expect(
    "Missing symbols list! Generate using tools/napi/generate_symbols_lists.js",
  );

  #[cfg(target_os = "windows")]
  println!(
    "cargo:rustc-link-arg-bin=deno=/DEF:{}",
    symbols_path.display()
  );

  #[cfg(target_os = "macos")]
  println!(
    "cargo:rustc-link-arg-bin=deno=-Wl,-exported_symbols_list,{}",
    symbols_path.display()
  );

  #[cfg(target_os = "linux")]
  {
    let ver = glibc_version::get_version().unwrap();

    // If a custom compiler is set, the glibc version is not reliable.
    // Here, we assume that if a custom compiler is used, that it will be modern enough to support a dynamic symbol list.
    if env::var("CC").is_err() && ver.major <= 2 && ver.minor < 35 {
      println!("cargo:warning=Compiling with all symbols exported, this will result in a larger binary. Please use glibc 2.35 or later for an optimised build.");
      println!("cargo:rustc-link-arg-bin=deno=-rdynamic");
    } else {
      println!(
        "cargo:rustc-link-arg-bin=deno=-Wl,--export-dynamic-symbol-list={}",
        symbols_path.display()
      );
    }
  }

  // To debug snapshot issues uncomment:
  // op_fetch_asset::trace_serializer();

  if let Ok(c) = env::var("DENO_CANARY") {
    println!("cargo:rustc-env=DENO_CANARY={c}");
  }
  println!("cargo:rerun-if-env-changed=DENO_CANARY");

  println!("cargo:rustc-env=GIT_COMMIT_HASH={}", git_commit_hash());
  println!("cargo:rerun-if-env-changed=GIT_COMMIT_HASH");

  println!("cargo:rustc-env=TS_VERSION={}", ts::version());
  println!("cargo:rerun-if-env-changed=TS_VERSION");

  println!("cargo:rustc-env=TARGET={}", env::var("TARGET").unwrap());
  println!("cargo:rustc-env=PROFILE={}", env::var("PROFILE").unwrap());

  let c = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
  let o = PathBuf::from(env::var_os("OUT_DIR").unwrap());

  let compiler_snapshot_path = o.join("COMPILER_SNAPSHOT.bin");
  ts::create_compiler_snapshot(compiler_snapshot_path, &c);

  let cli_snapshot_path = o.join("CLI_SNAPSHOT.bin");
  create_cli_snapshot(cli_snapshot_path);

  #[cfg(target_os = "windows")]
  {
    let mut res = winres::WindowsResource::new();
    res.set_icon("deno.ico");
    res.set_language(winapi::um::winnt::MAKELANGID(
      winapi::um::winnt::LANG_ENGLISH,
      winapi::um::winnt::SUBLANG_ENGLISH_US,
    ));
    res.compile().unwrap();
  }
}

fn deno_webgpu_get_declaration() -> PathBuf {
  let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
  manifest_dir
    .join("tsc")
    .join("dts")
    .join("lib.deno_webgpu.d.ts")
}
