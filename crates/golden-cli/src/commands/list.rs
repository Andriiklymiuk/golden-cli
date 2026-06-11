//! `golden list`: enumerate discovered collections and their requests as a tree.

use std::io::{self, Write};

use golden_core::model::{Collection, Item};

use crate::cli::ListArgs;
use crate::discovery::{discover, env_paths, expand_paths};
use crate::exit::FATAL;
use crate::filter::{prune_collection, Filter};
use crate::load::load;

/// Execute the list command. Returns the process exit code.
pub fn execute(args: &ListArgs, collections_override: &[String]) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return FATAL;
        }
    };

    let files = if args.paths.is_empty() {
        discover(&workspace, collections_override, env_paths())
    } else {
        expand_paths(&workspace, &args.paths)
    };

    if files.is_empty() {
        eprintln!("golden: no collections found");
        return FATAL;
    }

    let filter = match Filter::new(args.filter.as_deref()) {
        Ok(f) => f.with_methods(&args.method),
        Err(e) => {
            eprintln!("golden: invalid --filter glob: {e}");
            return FATAL;
        }
    };

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for file in &files {
        let mut loaded = match load(file) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("golden: {e}");
                return FATAL;
            }
        };
        prune_collection(&mut loaded.collection, &filter);
        if let Err(e) = print_collection(&mut out, &loaded.collection, file) {
            eprintln!("golden: {e}");
            return FATAL;
        }
    }
    0
}

fn print_collection(
    out: &mut dyn Write,
    collection: &Collection,
    path: &std::path::Path,
) -> io::Result<()> {
    writeln!(out, "{}  ({})", collection.info.name, path.display())?;
    print_items(out, &collection.item, 1)?;
    writeln!(out)
}

fn print_items(out: &mut dyn Write, items: &[Item], depth: usize) -> io::Result<()> {
    let indent = "  ".repeat(depth);
    for item in items {
        if item.is_folder() {
            writeln!(out, "{indent}{}/", item.name)?;
            if let Some(children) = &item.item {
                print_items(out, children, depth + 1)?;
            }
        } else if let Some(req) = &item.request {
            writeln!(
                out,
                "{indent}{} {}  {}",
                req.method,
                item.name,
                req.url.raw()
            )?;
        }
    }
    Ok(())
}
