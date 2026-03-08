// Entry point
pub const ENTRYPOINT: &str = "main";

// WASM module identifiers
pub const WASI_SNAPSHOT_MODULE: &str = "wasi_snapshot_preview1";
pub const WASI_MODULE_PREFIX: &str = "wasi:";
pub const WASI_CLI_RUN_EXPORT: &str = "wasi:cli/run@0.2.6#run";
pub const NEXUS_HOST_HTTP_MODULE: &str = "nexus:cli/nexus-host";
pub const NEXUS_HOST_HTTP_FUNC: &str = "host-http-request";
pub const MEMORY_EXPORT: &str = "memory";

// Custom section
pub const NEXUS_CAPABILITIES_SECTION: &str = "nexus:capabilities";

/// Runtime permission — parse-don't-validate enum for capability names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    Fs,
    Net,
    Console,
    Random,
    Clock,
    Proc,
    Env,
}

impl Permission {
    pub const ALL: [Permission; 7] = [
        Permission::Fs,
        Permission::Net,
        Permission::Console,
        Permission::Random,
        Permission::Clock,
        Permission::Proc,
        Permission::Env,
    ];

    /// Parse a permission type name (e.g. "PermFs") → Some(Permission::Fs)
    pub fn from_perm_name(name: &str) -> Option<Self> {
        match name {
            "PermFs" => Some(Permission::Fs),
            "PermNet" => Some(Permission::Net),
            "PermConsole" => Some(Permission::Console),
            "PermRandom" => Some(Permission::Random),
            "PermClock" => Some(Permission::Clock),
            "PermProc" => Some(Permission::Proc),
            "PermEnv" => Some(Permission::Env),
            _ => None,
        }
    }

    /// Parse a capability name (e.g. "Fs") → Some(Permission::Fs)
    pub fn from_cap_name(name: &str) -> Option<Self> {
        match name {
            "Fs" => Some(Permission::Fs),
            "Net" => Some(Permission::Net),
            "Console" => Some(Permission::Console),
            "Random" => Some(Permission::Random),
            "Clock" => Some(Permission::Clock),
            "Proc" => Some(Permission::Proc),
            "Env" => Some(Permission::Env),
            _ => None,
        }
    }

    /// The type name used in source code: "PermFs", "PermNet", etc.
    pub fn perm_name(&self) -> &'static str {
        match self {
            Permission::Fs => "PermFs",
            Permission::Net => "PermNet",
            Permission::Console => "PermConsole",
            Permission::Random => "PermRandom",
            Permission::Clock => "PermClock",
            Permission::Proc => "PermProc",
            Permission::Env => "PermEnv",
        }
    }

    /// The capability name used in wasm sections: "Fs", "Net", etc.
    pub fn cap_name(&self) -> &'static str {
        match self {
            Permission::Fs => "Fs",
            Permission::Net => "Net",
            Permission::Console => "Console",
            Permission::Random => "Random",
            Permission::Clock => "Clock",
            Permission::Proc => "Proc",
            Permission::Env => "Env",
        }
    }

    /// CLI flag: "--allow-fs", "--allow-net", etc.
    pub fn flag(&self) -> &'static str {
        match self {
            Permission::Fs => "--allow-fs",
            Permission::Net => "--allow-net",
            Permission::Console => "--allow-console",
            Permission::Random => "--allow-random",
            Permission::Clock => "--allow-clock",
            Permission::Proc => "--allow-proc",
            Permission::Env => "--allow-env",
        }
    }
}

pub fn is_preview2_wasi_module(module_name: &str) -> bool {
    module_name.starts_with(WASI_MODULE_PREFIX)
}
