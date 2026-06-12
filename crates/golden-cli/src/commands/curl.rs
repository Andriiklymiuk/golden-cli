//! `golden curl <collection> <request>` — print a curl command for one request.

use golden_core::curl::{generate, CurlOptions};
use golden_core::env::resolve;

use crate::cli::CurlArgs;
use crate::discovery::{discover, env_paths};
use crate::exit::FATAL;
use crate::load::load;

use super::send::select_request;

/// Execute the curl subcommand. Returns the process exit code.
pub fn execute(args: &CurlArgs) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return FATAL;
        }
    };

    // Support a direct file path OR collection discovery by name/stem.
    let path_candidate = std::path::Path::new(&args.collection);
    let loaded = if path_candidate.is_file() {
        match load(path_candidate) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("golden: {e}");
                return FATAL;
            }
        }
    } else {
        let files = discover(&workspace, &[], env_paths());
        if files.is_empty() {
            eprintln!("golden: no collections found");
            return FATAL;
        }
        let mut found = None;
        for file in &files {
            match load(file) {
                Ok(l) => {
                    let name_match = l.collection.info.name == args.collection;
                    let stem_match = l
                        .path
                        .file_stem()
                        .map(|s| s.to_string_lossy() == args.collection.as_str())
                        .unwrap_or(false);
                    if name_match || stem_match {
                        found = Some(l);
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("golden: {e}");
                    return FATAL;
                }
            }
        }
        match found {
            Some(l) => l,
            None => {
                eprintln!("golden: collection '{}' not found", args.collection);
                return FATAL;
            }
        }
    };

    let request = match select_request(&loaded.collection, &args.request, args.index) {
        Ok(m) => m.request,
        Err(e) => {
            eprintln!("golden: {e} in collection '{}'", args.collection);
            return FATAL;
        }
    };

    let scopes = resolve(
        &loaded.workspace,
        &loaded.collections_root,
        &loaded.collection.variable,
    );

    let opts = CurlOptions {
        follow_redirects: args.follow,
        include_headers: args.include,
        silent: args.silent,
        insecure: args.insecure,
        fail: args.fail,
        compressed: args.compressed,
        timing: args.timing,
        file_download: args.download,
        mask: args.mask,
    };

    let line = generate(request, scopes.as_map(), &opts);
    println!("{line}");
    0
}
