use ignore::gitignore::Gitignore;
use glob::Pattern;
use std::path::PathBuf;

pub fn handle_event(
    w: &PathBuf,
    filter: &Filter,
) -> bool {
    if filter.watched_files.contains(&w.to_string_lossy().as_ref()) {
        return true;
    }
    for ignore in &filter.ignore_globs {
        if ignore.matches(w.to_string_lossy().as_ref()) {
            return false;
        }
    }
    if !filter.extensions.is_empty() {
        return match w.extension() {
            Some(ext) => {
                filter.extensions.contains(&ext.to_string_lossy().as_ref())
            }
            None => {
                let z = w.file_name().unwrap().to_str().unwrap();
                if z.starts_with(".") {
                    filter.extensions.contains(&&z[1..])
                } else {
                    false
                }
            }
        };
    }
    if let Some(gitignore) = &filter.gitignore {
        if gitignore.matched_path_or_any_parents(&w.to_string_lossy().as_ref(), w.is_dir()).is_ignore() {
            return false;
        }
    }
    if let Some(global_ignore) = &filter.global_gitignore {
        if global_ignore.matched_path_or_any_parents(&w.to_string_lossy().as_ref(), w.is_dir()).is_ignore() {
            return false;
        }
    }
    true
}


#[derive(Debug)]
pub struct Filter<'a> {
    pub watched_files: Vec<&'a str>,
    pub extensions: Vec<&'a str>,
    pub gitignore: Option<Gitignore>,
    pub global_gitignore: Option<Gitignore>,
    pub ignore_globs: Vec<Pattern>,
}

impl<'a> Filter<'a> {
    // pub fn new() -> Self {
    //     Filter {
    //         watched_files: Vec::new(),
    //         extensions: Vec::new(),
    //         gitignore: None,
    //         global_gitignore: None,
    //         ignore_globs: Vec::new(),
    //     }
    // }
}


pub fn find_project_gitignore() -> Option<Gitignore> {
    let mut path = PathBuf::from(".");
    loop {
        let gitignore_path = path.join(".gitignore");
        if gitignore_path.exists() {
            let (ignore, _) = Gitignore::new(gitignore_path);
            return Some(ignore);
        }
        if path.parent().is_none() || path.join(".git").exists() {
            return None;
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    
    use ignore::gitignore::GitignoreBuilder;
    use super::*;

    #[test]
    fn test_extension() {
        let mut filter = Filter::new();
        filter.extensions.push("rs");
        let p = PathBuf::from("/Users/debug/.fingerprint/server2-66aa47d134ef7589/invoked.timestamp");
        assert_eq!(handle_event(&p, &filter), false, ".timestamp ignored when watching .rs files");
    }

    #[test]
    fn test_gitignore() {
        let mut filter = Filter::new();
        let root = PathBuf::from("/Users/kurt/work/server/");
        let mut ignore = GitignoreBuilder::new(&root);
            ignore.add_line(Some(root), "/target").unwrap();
        let ignore = ignore.build().unwrap();
        filter.gitignore = Some(ignore);
        let p = PathBuf::from("/Users/kurt/work/server/target/debug/.fingerprint/foo.rs");
        assert_eq!(handle_event(&p, &filter), false, "gitignore should understand paths relative to project root");
    }

}