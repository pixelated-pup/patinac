use std::{path::PathBuf, str::FromStr};

use inkwell::targets::TargetTriple;

/// What to produce, if anything.
#[derive(PartialEq, Eq)]
pub enum CodegenMode {
    /// Dump the LLVM IR to stderr.
    IRDump,
    /// Emit an object file at the given path.
    Emit(PathBuf),
    /// Run verification checks but do nothing else (for testing).
    Silent,
}

/// What level of optimisation to use.
///
/// Currently directly corresponds to the LLVM optimisation levels of the same names.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OptLevel {
    /// `-O0`. No optimisation.
    #[default]
    O0 = 0,
    /// `-O1`.
    O1 = 1,
    /// `-O2`.
    O2 = 2,
    /// `-O3`. Full optimisations (minus LTO).
    O3 = 3,
}

impl FromStr for OptLevel {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "0" => Ok(Self::O0),
            "1" => Ok(Self::O1),
            "2" => Ok(Self::O2),
            "3" => Ok(Self::O3),
            _ => Err(r#"expected "0", "1", "2", or "3""#),
        }
    }
}

impl OptLevel {
    /// Converts the optimisation level into a string of LLVM optimisation passes, as expected by `opt`.
    #[expect(clippy::as_conversions, reason = "accessing enum discriminant")]
    pub fn opt_string(self) -> String {
        match self {
            Self::O0 | Self::O1 | Self::O2 | Self::O3 => {
                format!("default<O{}>", self as u8)
            }
        }
    }
}

/// Supported compilation targets.
#[derive(Debug, Clone, Copy)]
#[expect(clippy::doc_markdown, reason = "false positive")]
pub enum Target {
    /// Windows on x86_64 using the MSVC toolchain (`x86_64-pc-windows-msvc`).
    ///
    /// Requires VS2022+. Uses the `x64` calling convention for C functions. Produces PE binaries.
    X86_64WindowsMSVC,
    /// Windows on x86_64 using the GNU toolchain (`x86_64-pc-windows-gnu`).
    ///
    /// Uses the `x64` calling convention for C functions. Produces PE binaries.
    X86_64WindowsGNU,
    /// Windows on ARM64/aarch64 using the MSVC toolchain (`aarch64-pc-windows-msvc`).
    ///
    /// Requires VS2022+. Uses the `ARM64` calling convention for C functions. Produces PE binaries.
    Arm64WindowsMSVC,
    /// Linux on x86_64 using the GNU toolchain (`x86_64-unknown-linux-gnu`).
    X86_64LinuxGNU,
    /// Linux on ARM64/aarch64 using the GNU toolchain (`aarch64-unknown-linux-gnu`).
    Arm64Linux,
    /// MacOS on ARM64/aarch64 (`aarch64-apple-darwin`).
    ///
    /// Requires Xcode??. Produces Mach-O binaries.
    Arm64Mac,
}

impl Target {
    /// Returns the target corresponding to the host platform, if there is one.
    pub const fn host() -> Option<Self> {
        cfg_select! {
            all(target_arch = "x86_64", target_os = "windows") => Some(Self::X86_64Windows),
            all(target_arch = "x86_64", target_os = "linux") => Some(Self::X86_64LinuxGNU),
            all(target_arch = "aarch64", target_os = "windows") => Some(Self::Arm64Windows),
            all(target_arch = "aarch64", target_os = "linux") => Some(Self::Arm64Linux),
            all(target_arch = "aarch64", target_os = "macos") => Some(Self::Arm64Mac),
            _ => None
        }
    }

    /// Returns the LLVM target triple corresponding to this target.
    pub fn triple(self) -> TargetTriple {
        let name = match self {
            Self::X86_64WindowsMSVC => "x86_64-pc-windows-msvc",
            Self::X86_64WindowsGNU => "x86_64-pc-windows-gnu",
            Self::Arm64WindowsMSVC => "aarch64-pc-windows-msvc",
            Self::X86_64LinuxGNU => "x86_64-unknown-linux-gnu",
            Self::Arm64Linux => "aarch64-unknown-linux-gnu",
            Self::Arm64Mac => "aarch64-apple-darwin",
        };
        TargetTriple::create(name)
    }
}

impl Default for Target {
    fn default() -> Self {
        Self::host().unwrap_or(Self::X86_64LinuxGNU)
    }
}

#[test]
fn validate_triples() {
    use inkwell::targets::{InitializationConfig, Target as LLVMTarget};

    LLVMTarget::initialize_all(&InitializationConfig::default());

    let targets = [
        Target::X86_64WindowsMSVC,
        Target::X86_64LinuxGNU,
        Target::Arm64WindowsMSVC,
        Target::Arm64Linux,
        Target::Arm64Mac,
    ];

    for target in targets {
        LLVMTarget::from_triple(&target.triple()).unwrap();
    }
}
