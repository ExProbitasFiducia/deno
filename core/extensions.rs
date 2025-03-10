// Copyright 2018-2023 the Deno authors. All rights reserved. MIT license.
use crate::OpState;
use anyhow::Error;
use std::cell::RefCell;
use std::rc::Rc;
use std::task::Context;
use v8::fast_api::FastFunction;

#[derive(Clone, Debug)]
pub struct ExtensionFileSource {
  pub specifier: String,
  pub code: &'static str,
}
pub type OpFnRef = v8::FunctionCallback;
pub type OpMiddlewareFn = dyn Fn(OpDecl) -> OpDecl;
pub type OpStateFn = dyn Fn(&mut OpState) -> Result<(), Error>;
pub type OpEventLoopFn = dyn Fn(Rc<RefCell<OpState>>, &mut Context) -> bool;

pub struct OpDecl {
  pub name: &'static str,
  pub v8_fn_ptr: OpFnRef,
  pub enabled: bool,
  pub is_async: bool,
  pub is_unstable: bool,
  /// V8 argument count. Used as an optimization
  /// hint by `core.initalizeAsyncOps`.
  pub argc: usize,
  pub is_v8: bool,
  pub fast_fn: Option<Box<dyn FastFunction>>,
}

impl OpDecl {
  pub fn enabled(self, enabled: bool) -> Self {
    Self { enabled, ..self }
  }

  pub fn disable(self) -> Self {
    self.enabled(false)
  }
}

#[derive(Default)]
pub struct Extension {
  js_files: Option<Vec<ExtensionFileSource>>,
  esm_files: Option<Vec<ExtensionFileSource>>,
  ops: Option<Vec<OpDecl>>,
  opstate_fn: Option<Box<OpStateFn>>,
  middleware_fn: Option<Box<OpMiddlewareFn>>,
  event_loop_middleware: Option<Box<OpEventLoopFn>>,
  initialized: bool,
  enabled: bool,
  name: &'static str,
  deps: Option<Vec<&'static str>>,
}

// Note: this used to be a trait, but we "downgraded" it to a single concrete type
// for the initial iteration, it will likely become a trait in the future
impl Extension {
  pub fn builder(name: &'static str) -> ExtensionBuilder {
    ExtensionBuilder {
      name,
      ..Default::default()
    }
  }

  /// Check if dependencies have been loaded, and errors if either:
  /// - The extension is depending on itself or an extension with the same name.
  /// - A dependency hasn't been loaded yet.
  pub fn check_dependencies(&self, previous_exts: &[&mut Extension]) {
    if let Some(deps) = &self.deps {
      'dep_loop: for dep in deps {
        if dep == &self.name {
          panic!("Extension '{}' is either depending on itself or there is another extension with the same name", self.name);
        }

        for ext in previous_exts {
          if dep == &ext.name {
            continue 'dep_loop;
          }
        }

        panic!("Extension '{}' is missing dependency '{dep}'", self.name);
      }
    }
  }

  /// returns JS source code to be loaded into the isolate (either at snapshotting,
  /// or at startup).  as a vector of a tuple of the file name, and the source code.
  pub fn get_js_sources(&self) -> &[ExtensionFileSource] {
    match &self.js_files {
      Some(files) => files,
      None => &[],
    }
  }

  pub fn get_esm_sources(&self) -> &[ExtensionFileSource] {
    match &self.esm_files {
      Some(files) => files,
      None => &[],
    }
  }

  /// Called at JsRuntime startup to initialize ops in the isolate.
  pub fn init_ops(&mut self) -> Option<Vec<OpDecl>> {
    // TODO(@AaronO): maybe make op registration idempotent
    if self.initialized {
      panic!("init_ops called twice: not idempotent or correct");
    }
    self.initialized = true;

    let mut ops = self.ops.take()?;
    for op in ops.iter_mut() {
      op.enabled = self.enabled && op.enabled;
    }
    Some(ops)
  }

  /// Allows setting up the initial op-state of an isolate at startup.
  pub fn init_state(&self, state: &mut OpState) -> Result<(), Error> {
    match &self.opstate_fn {
      Some(ofn) => ofn(state),
      None => Ok(()),
    }
  }

  /// init_middleware lets us middleware op registrations, it's called before init_ops
  pub fn init_middleware(&mut self) -> Option<Box<OpMiddlewareFn>> {
    self.middleware_fn.take()
  }

  pub fn init_event_loop_middleware(&mut self) -> Option<Box<OpEventLoopFn>> {
    self.event_loop_middleware.take()
  }

  pub fn run_event_loop_middleware(
    &self,
    op_state_rc: Rc<RefCell<OpState>>,
    cx: &mut Context,
  ) -> bool {
    self
      .event_loop_middleware
      .as_ref()
      .map(|f| f(op_state_rc, cx))
      .unwrap_or(false)
  }

  pub fn enabled(self, enabled: bool) -> Self {
    Self { enabled, ..self }
  }

  pub fn disable(self) -> Self {
    self.enabled(false)
  }
}

// Provides a convenient builder pattern to declare Extensions
#[derive(Default)]
pub struct ExtensionBuilder {
  js: Vec<ExtensionFileSource>,
  esm: Vec<ExtensionFileSource>,
  ops: Vec<OpDecl>,
  state: Option<Box<OpStateFn>>,
  middleware: Option<Box<OpMiddlewareFn>>,
  event_loop_middleware: Option<Box<OpEventLoopFn>>,
  name: &'static str,
  deps: Vec<&'static str>,
}

impl ExtensionBuilder {
  pub fn dependencies(&mut self, dependencies: Vec<&'static str>) -> &mut Self {
    self.deps.extend(dependencies);
    self
  }

  pub fn js(&mut self, js_files: Vec<ExtensionFileSource>) -> &mut Self {
    let js_files =
      js_files.into_iter().map(|file_source| ExtensionFileSource {
        specifier: format!("internal:{}/{}", self.name, file_source.specifier),
        code: file_source.code,
      });
    self.js.extend(js_files);
    self
  }

  pub fn esm(&mut self, esm_files: Vec<ExtensionFileSource>) -> &mut Self {
    let esm_files =
      esm_files
        .into_iter()
        .map(|file_source| ExtensionFileSource {
          specifier: format!(
            "internal:{}/{}",
            self.name, file_source.specifier
          ),
          code: file_source.code,
        });
    self.esm.extend(esm_files);
    self
  }

  pub fn ops(&mut self, ops: Vec<OpDecl>) -> &mut Self {
    self.ops.extend(ops);
    self
  }

  pub fn state<F>(&mut self, opstate_fn: F) -> &mut Self
  where
    F: Fn(&mut OpState) -> Result<(), Error> + 'static,
  {
    self.state = Some(Box::new(opstate_fn));
    self
  }

  pub fn middleware<F>(&mut self, middleware_fn: F) -> &mut Self
  where
    F: Fn(OpDecl) -> OpDecl + 'static,
  {
    self.middleware = Some(Box::new(middleware_fn));
    self
  }

  pub fn event_loop_middleware<F>(&mut self, middleware_fn: F) -> &mut Self
  where
    F: Fn(Rc<RefCell<OpState>>, &mut Context) -> bool + 'static,
  {
    self.event_loop_middleware = Some(Box::new(middleware_fn));
    self
  }

  pub fn build(&mut self) -> Extension {
    let js_files = Some(std::mem::take(&mut self.js));
    let esm_files = Some(std::mem::take(&mut self.esm));
    let ops = Some(std::mem::take(&mut self.ops));
    let deps = Some(std::mem::take(&mut self.deps));
    Extension {
      js_files,
      esm_files,
      ops,
      opstate_fn: self.state.take(),
      middleware_fn: self.middleware.take(),
      event_loop_middleware: self.event_loop_middleware.take(),
      initialized: false,
      enabled: true,
      name: self.name,
      deps,
    }
  }
}

/// Helps embed JS files in an extension. Returns a vector of
/// `ExtensionFileSource`, that represent the filename and source code.
///
/// Example:
/// ```ignore
/// include_js_files!(
///   "01_hello.js",
///   "02_goodbye.js",
/// )
/// ```
#[macro_export]
macro_rules! include_js_files {
  ($($file:literal,)+) => {
    vec![
      $($crate::ExtensionFileSource {
        specifier: $file.to_string(),
        code: include_str!($file),
      },)+
    ]
  };
}

/// Helps embed JS files in an extension. Returns a vector of
/// `ExtensionFileSource`, that represent the filename and source code.
/// Additional "dir" option is required, that specifies which directory in the
/// crate root contains the listed files. "dir" option will be prepended to
/// each file name.
///
/// Example:
/// ```ignore
/// include_js_files_dir!(
///   dir "example",
///   "01_hello.js",
///   "02_goodbye.js",
/// )
/// ```
#[macro_export]
macro_rules! include_js_files_dir {
  (dir $dir:literal, $($file:literal,)+) => {
    vec![
      $($crate::ExtensionFileSource {
        specifier: concat!($dir, "/", $file).to_string(),
        code: include_str!(concat!($dir, "/", $file)),
      },)+
    ]
  };
}
