use ignore::gitignore::Gitignore;
use glob::Pattern;
use std::path::{Path, PathBuf};

pub fn handle_event(
    p: &Path,
    filter: &Filter,
) -> bool {
    if filter.watched_files.iter().any(|w| w == p) {
        return true;
    }
    let Ok(p) = p.strip_prefix(&filter.working_dir) else {
        return false;
    };
    for ignore in &filter.ignore_globs {
        if ignore.matches_path(&p) {
            return false;
        }
    }
    if !filter.extensions.is_empty() {
        return p.components()
            .last()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap()
            .splitn(2, '.')
            .last()
            .map(|ext| filter.extensions.iter().any(|e| e == ext))
            .unwrap_or(false);
    }
    if let Some(gitignore) = &filter.gitignore {
        if gitignore.matched_path_or_any_parents(&p.to_string_lossy().as_ref(), p.is_dir()).is_ignore() {
            return false;
        }
    }
    if let Some(global_ignore) = &filter.global_gitignore {
        if global_ignore.matched_path_or_any_parents(&p.to_string_lossy().as_ref(), p.is_dir()).is_ignore() {
            return false;
        }
    }
    true
}


#[derive(Debug)]
pub struct Filter {
    pub working_dir: PathBuf,
    /// Must be canonical (absolute) paths
    pub watched_files: Vec<PathBuf>,
    pub extensions: Vec<String>,
    pub gitignore: Option<Gitignore>,
    pub global_gitignore: Option<Gitignore>,
    pub ignore_globs: Vec<Pattern>,
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

        filter.extensions.push("ts");
        let p = PathBuf::from("foo/bar.d.ts");
        assert_eq!(handle_event(&p, &filter), false, "handle two file extensions");
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

    #[test]
    fn test_ignore() {
        let mut filter = Filter::new();
        filter.ignore_globs.push(Pattern::new("**/.DS_Store").unwrap());
        let path = PathBuf::from("/Users/kurt/work/server/dist/.DS_Store");
        assert_eq!(handle_event(&path, &filter), false, "ignore globs should match");

        filter.ignore_globs.push(Pattern::new("**/.git").unwrap());
        let path = PathBuf::from("/Users/kurt/work/server/.git");
        assert_eq!(handle_event(&path, &filter), false, "ignore globs should match");

        filter.ignore_globs.push(Pattern::new("dist/*").unwrap());
        let path = PathBuf::from("dist/foo.ts");
        assert_eq!(handle_event(&path, &filter), false, "ignore globs should match");
    }

}
