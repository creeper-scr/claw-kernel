use std::path::PathBuf;

/// Returns default skill search directories (lowest to highest priority).
///
/// 1. `<claw install dir>/skills` — builtin skills (lowest priority)
/// 2. `~/.claw/skills` — user-global skills
/// 3. `./skills` — workspace-local skills (highest priority)
pub fn default_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. Builtin skills (next to binary)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let builtin = parent.join("skills");
            dirs.push(builtin);
        }
    }

    // 2. User global: ~/.claw/skills
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".claw").join("skills"));
    }

    // 3. Workspace local: ./skills
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join("skills"));
    }

    dirs
}
