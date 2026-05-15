use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QemuTcgArch {
    Armv7a,
}

impl QemuTcgArch {
    fn clang_args(self) -> &'static [&'static str] {
        match self {
            Self::Armv7a => &["-march=armv7-a", "-mfpu=neon", "-mfloat-abi=softfp"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuTcgExit {
    pub code: Option<i32>,
    pub status_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QemuTcgError {
    MissingTool(&'static str),
    TempDir(String),
    WriteSource(String),
    ClangFailed(String),
    QemuFailed(String),
    Time(String),
}

impl fmt::Display for QemuTcgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTool(tool) => write!(f, "required tool not found: {tool}"),
            Self::TempDir(err) => write!(f, "failed to create temporary directory: {err}"),
            Self::WriteSource(err) => write!(f, "failed to write temporary source: {err}"),
            Self::ClangFailed(err) => write!(f, "clang failed: {err}"),
            Self::QemuFailed(err) => write!(f, "qemu-arm failed: {err}"),
            Self::Time(err) => write!(f, "failed to build temporary timestamp: {err}"),
        }
    }
}

impl std::error::Error for QemuTcgError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuArmTcgRunner {
    qemu_arm: String,
    clang: String,
}

impl QemuArmTcgRunner {
    pub fn probe() -> Result<Self, QemuTcgError> {
        let runner = Self {
            qemu_arm: "qemu-arm".to_string(),
            clang: "clang".to_string(),
        };
        runner.check_tool(&runner.qemu_arm, "qemu-arm")?;
        runner.check_tool(&runner.clang, "clang")?;
        Ok(runner)
    }

    pub fn run_linux_exit_asm(
        &self,
        arch: QemuTcgArch,
        asm: &str,
    ) -> Result<QemuTcgExit, QemuTcgError> {
        let dir = self.temp_dir()?;
        let asm_path = dir.join("test.S");
        let elf_path = dir.join("test");
        fs::write(&asm_path, asm).map_err(|err| QemuTcgError::WriteSource(err.to_string()))?;

        let output = Command::new(&self.clang)
            .arg("--target=arm-linux-gnueabi")
            .args(arch.clang_args())
            .arg("-nostdlib")
            .arg("-static")
            .arg("-fuse-ld=lld")
            .arg("-Wl,-e,_start")
            .arg(&asm_path)
            .arg("-o")
            .arg(&elf_path)
            .output()
            .map_err(|err| QemuTcgError::ClangFailed(err.to_string()))?;
        if !output.status.success() {
            return Err(QemuTcgError::ClangFailed(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        let status = Command::new(&self.qemu_arm)
            .arg(&elf_path)
            .status()
            .map_err(|err| QemuTcgError::QemuFailed(err.to_string()))?;
        Ok(QemuTcgExit {
            code: status.code(),
            status_text: status.to_string(),
        })
    }

    fn check_tool(&self, command: &str, label: &'static str) -> Result<(), QemuTcgError> {
        Command::new(command)
            .arg("--version")
            .output()
            .map(|_| ())
            .map_err(|_| QemuTcgError::MissingTool(label))
    }

    fn temp_dir(&self) -> Result<PathBuf, QemuTcgError> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| QemuTcgError::Time(err.to_string()))?
            .as_nanos();
        let mut dir = std::env::temp_dir();
        dir.push(format!("aemu-qemu-tcg-{stamp}"));
        fs::create_dir_all(&dir).map_err(|err| QemuTcgError::TempDir(err.to_string()))?;
        Ok(dir)
    }
}

pub fn armv7a_smoke_arm_words() -> &'static [u32] {
    &[
        0xe3a0_002a, // mov r0, #42
        0xe3a0_7001, // mov r7, #1
        0xef00_0000, // svc #0
    ]
}

pub fn armv7a_smoke_program() -> String {
    let mut asm = ".syntax unified\n\
                   .arch armv7-a\n\
                   .fpu neon\n\
                   .text\n\
                   .global _start\n\
                   _start:\n"
        .to_string();
    for word in armv7a_smoke_arm_words() {
        asm.push_str(&format!("  .word 0x{word:08x}\n"));
    }
    asm
}
