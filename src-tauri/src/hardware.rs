//! What this computer can run: CPU, memory, GPU and free disk space, and the Qwen model that
//! fits it best. The recommendation is a pure function of the report so it can be tested.

use serde::Serialize;
use std::path::Path;
use sysinfo::{Disks, MemoryRefreshKind, RefreshKind, System};

const GB: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(not(windows), allow(dead_code))] // only DXGI reports these vendors
pub enum GpuKind {
    /// Apple Silicon: the GPU shares the main memory.
    Apple,
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    pub kind: GpuKind,
    /// Dedicated video memory; None when shared with the system.
    pub vram_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareReport {
    pub os: String,
    pub arch: String,
    pub cpu: String,
    pub cores: usize,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub gpus: Vec<Gpu>,
    /// Free space on the disk holding the app data (models and index).
    pub free_disk_bytes: u64,
}

pub fn detect(data_dir: &Path) -> HardwareReport {
    let sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_memory(MemoryRefreshKind::nothing().with_ram())
            .with_cpu(sysinfo::CpuRefreshKind::nothing()),
    );
    let cpu = sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default();
    let cores = System::physical_core_count().unwrap_or_else(|| sys.cpus().len().max(1));
    HardwareReport {
        os: System::long_os_version().unwrap_or_else(|| std::env::consts::OS.into()),
        arch: std::env::consts::ARCH.into(),
        gpus: gpus(&cpu),
        cpu,
        cores,
        total_memory_bytes: sys.total_memory(),
        available_memory_bytes: sys.available_memory(),
        free_disk_bytes: free_disk(data_dir),
    }
}

/// Free space on the disk whose mount point is the longest prefix of `dir`.
pub fn free_disk(dir: &Path) -> u64 {
    // Without Windows' \\?\ prefix, which no mount point (`C:\`) is a prefix of.
    let dir = crate::access::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|d| dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
        .unwrap_or(0)
}

#[cfg(target_os = "macos")]
fn gpus(cpu: &str) -> Vec<Gpu> {
    if cfg!(target_arch = "aarch64") {
        let name = if cpu.is_empty() { "Apple Silicon".into() } else { format!("{cpu} GPU") };
        vec![Gpu { name, kind: GpuKind::Apple, vram_bytes: None }]
    } else {
        // Intel Macs: the engine runs on the CPU.
        vec![]
    }
}

#[cfg(windows)]
fn gpus(_cpu: &str) -> Vec<Gpu> {
    windows_gpu::adapters()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn gpus(_cpu: &str) -> Vec<Gpu> {
    vec![]
}

#[cfg(windows)]
mod windows_gpu {
    use super::{Gpu, GpuKind};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};

    /// Hardware adapters as reported by DXGI (no driver tools needed).
    pub fn adapters() -> Vec<Gpu> {
        let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
            return vec![];
        };
        let mut out = Vec::new();
        for i in 0.. {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else { break };
            let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
            if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            let kind = match desc.VendorId {
                0x10DE => GpuKind::Nvidia,
                0x1002 => GpuKind::Amd,
                0x8086 => GpuKind::Intel,
                _ => GpuKind::Other,
            };
            out.push(Gpu {
                name: String::from_utf16_lossy(&desc.Description[..len]),
                kind,
                vram_bytes: Some(desc.DedicatedVideoMemory as u64),
            });
        }
        out
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelOption {
    pub tag: &'static str,
    pub label: &'static str,
    /// Approximate download size; the exact size depends on the build the engine picks.
    pub download_bytes: u64,
    /// Memory budget at which this model runs comfortably.
    pub min_budget_bytes: u64,
    pub description: &'static str,
}

const fn gb(x: f64) -> u64 {
    (x * GB) as u64
}

/// A model's download in GB: the Apple Silicon (MLX) build macOS gets, or the GGUF build the
/// Windows engine runs (its size in Ollama's registry, checked 11.11.2026).
const fn download(mlx: f64, gguf: f64) -> u64 {
    gb(if cfg!(windows) { gguf } else { mlx })
}

/// The chat models, smallest first by parameters; the app recommends the largest that fits.
/// Qwen sizes are the upper end listed on ollama.com (checked 30.10.2026). Gemma's size is its
/// Apple Silicon download, and its memory need is Qwen 3.5 9B's scaled by their measured peak
/// use with about 23,000 tokens of context (11.0 against 13.0 GB on an M3 Pro, 03.11.2026).
const ALL_CHAT_MODELS: &[ModelOption] = &[
    ModelOption {
        tag: "qwen3.5:0.8b",
        label: "Qwen 3.5 0.8B",
        download_bytes: download(1.3, 1.3),
        min_budget_bytes: 0,
        description: "Smallest and fastest. For computers with little memory; answers are simpler.",
    },
    ModelOption {
        tag: "qwen3.5:2b",
        label: "Qwen 3.5 2B",
        download_bytes: download(3.1, 2.5),
        min_budget_bytes: gb(3.5),
        description: "Light. Good for short questions and summaries.",
    },
    ModelOption {
        tag: "qwen3.5:4b",
        label: "Qwen 3.5 4B",
        download_bytes: download(4.0, 3.1),
        min_budget_bytes: gb(5.5),
        description: "Balanced. Reliable answers about documents on most laptops.",
    },
    ModelOption {
        tag: "qwen3.5:9b",
        label: "Qwen 3.5 9B",
        download_bytes: download(7.6, 6.2),
        min_budget_bytes: gb(10.0),
        description: "Stronger reasoning and better document analysis.",
    },
    ModelOption {
        tag: "gemma4:12b",
        label: "Gemma 4 12B",
        download_bytes: download(7.2, 7.5),
        min_budget_bytes: gb(8.5),
        description: "Made by Google. Larger than the 9B, yet needs less memory.",
    },
    ModelOption {
        tag: "qwen3.5:35b-a3b",
        label: "Qwen 3.5 35B-A3B",
        download_bytes: download(22.0, 20.6),
        min_budget_bytes: gb(26.0),
        description: "Most capable. Needs a workstation with plenty of memory.",
    },
];

/// How many of the smallest chat models Windows offers: those whose GGUF build is in the model
/// library (published with `--tag-suffix -gguf`, see docs/MODELS.md). They are also the largest a
/// computer without a big graphics card is recommended. Raise it once the next build is published.
const WINDOWS_MODELS: usize = 3;

/// The chat models this system offers: all of them on macOS, the smallest ones on Windows.
pub const CHAT_MODELS: &[ModelOption] =
    if cfg!(windows) { ALL_CHAT_MODELS.split_at(WINDOWS_MODELS).0 } else { ALL_CHAT_MODELS };

/// Embedding model for document search. Setup does not download it, so the disk space a model
/// needs leaves it out.
pub const EMBED_MODEL: ModelOption = ModelOption {
    tag: "qwen3-embedding:0.6b",
    label: "Qwen3 Embedding 0.6B",
    download_bytes: 639 * 1024 * 1024,
    min_budget_bytes: 0,
    description: "Finds the passages in your files that answer a question.",
};

/// Space kept free beyond the downloads, for the search index and temporary files.
pub const DISK_HEADROOM: u64 = gb(2.0);

/// Whether `free` bytes of disk are enough for `model`, of which `downloaded` bytes are on disk
/// already (an interrupted download that resumes).
pub fn fits_disk(model: &ModelOption, free: u64, downloaded: u64) -> bool {
    free.saturating_add(downloaded) >= model.download_bytes + DISK_HEADROOM
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub tag: &'static str,
    pub reason: String,
    pub budget_bytes: u64,
    /// Every model, with whether it fits this computer's memory and disk.
    pub options: Vec<OptionFit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionFit {
    #[serde(flatten)]
    pub model: ModelOption,
    pub fits_memory: bool,
    pub fits_disk: bool,
}

/// What runs the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Accelerator {
    Apple,
    Nvidia,
    Cpu,
}

impl Accelerator {
    fn explain(self) -> &'static str {
        match self {
            Self::Apple => "Apple Silicon shares its memory with the GPU",
            Self::Nvidia => "the NVIDIA graphics card runs the model",
            Self::Cpu => "the processor runs the model",
        }
    }
}

/// Memory the model may use: most of Apple Silicon's unified memory, a dedicated NVIDIA GPU's
/// memory, or half of the RAM when the CPU does the work.
pub fn memory_budget(hw: &HardwareReport) -> (u64, Accelerator) {
    if hw.gpus.iter().any(|g| g.kind == GpuKind::Apple) {
        return ((hw.total_memory_bytes as f64 * 0.65) as u64, Accelerator::Apple);
    }
    let vram = hw.gpus.iter().filter(|g| g.kind == GpuKind::Nvidia).filter_map(|g| g.vram_bytes).max().unwrap_or(0);
    if vram >= gb(4.0) {
        return (vram, Accelerator::Nvidia);
    }
    (hw.total_memory_bytes / 2, Accelerator::Cpu)
}

/// The largest model that fits this computer's memory and disk and runs at a pleasant speed;
/// with `largest`, no larger than that model (the edition's limit).
pub fn recommend(hw: &HardwareReport, largest: Option<&str>) -> Recommendation {
    let (budget, accel) = memory_budget(hw);
    let gpu = accel != Accelerator::Cpu;
    let options: Vec<OptionFit> = CHAT_MODELS
        .iter()
        .map(|m| OptionFit {
            model: m.clone(),
            fits_memory: budget >= m.min_budget_bytes,
            fits_disk: fits_disk(m, hw.free_disk_bytes, 0),
        })
        .collect();
    // Without a GPU, larger models are too slow to be pleasant; few cores make it worse.
    let cpu_cap = match (gpu, hw.cores) {
        (true, _) => CHAT_MODELS.len(),
        (false, c) if c < 4 => 2,
        (false, _) => 3,
    };
    let edition_cap =
        largest.and_then(|tag| CHAT_MODELS.iter().position(|m| m.tag == tag)).map_or(CHAT_MODELS.len(), |i| i + 1);
    let pick = options
        .iter()
        .take(cpu_cap.min(edition_cap))
        .rev()
        .find(|o| o.fits_memory && o.fits_disk)
        .map(|o| o.model.tag)
        .unwrap_or(CHAT_MODELS[0].tag);
    let label = CHAT_MODELS.iter().find(|m| m.tag == pick).map(|m| m.label).unwrap_or(pick);
    let mut reason = format!(
        "{label} fits about {:.0} GB of memory available for AI on this computer ({}).",
        budget as f64 / GB,
        accel.explain()
    );
    if !gpu {
        reason.push_str(" Without a supported graphics processor, smaller models answer faster.");
    }
    if !options.iter().any(|o| o.fits_disk) {
        reason.push_str(" Free up disk space first: even the smallest model does not fit.");
    }
    Recommendation { tag: pick, reason, budget_bytes: budget, options }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(ram_gb: f64, gpus: Vec<Gpu>, cores: usize, disk_gb: f64) -> HardwareReport {
        HardwareReport {
            os: "test".into(),
            arch: "test".into(),
            cpu: "test".into(),
            cores,
            total_memory_bytes: gb(ram_gb),
            available_memory_bytes: gb(ram_gb / 2.0),
            gpus,
            free_disk_bytes: gb(disk_gb),
        }
    }

    fn apple() -> Vec<Gpu> {
        vec![Gpu { name: "Apple M3 GPU".into(), kind: GpuKind::Apple, vram_bytes: None }]
    }

    fn nvidia(vram_gb: f64) -> Vec<Gpu> {
        vec![Gpu { name: "RTX".into(), kind: GpuKind::Nvidia, vram_bytes: Some(gb(vram_gb)) }]
    }

    #[test]
    fn recommends_by_memory_budget() {
        let cases = [
            (machine(8.0, apple(), 8, 500.0), "qwen3.5:2b"),
            // Gemma 4 12B is larger than Qwen 3.5 9B and needs less memory, so it wins where both fit.
            (machine(16.0, apple(), 8, 500.0), "gemma4:12b"),
            (machine(18.0, apple(), 11, 500.0), "gemma4:12b"),
            (machine(64.0, apple(), 12, 500.0), "qwen3.5:35b-a3b"),
            (machine(32.0, nvidia(8.0), 8, 500.0), "qwen3.5:4b"),
            (machine(32.0, nvidia(12.0), 8, 500.0), "gemma4:12b"),
            (machine(16.0, vec![], 8, 500.0), "qwen3.5:4b"),
            (machine(64.0, vec![], 8, 500.0), "qwen3.5:4b"),
            (machine(64.0, vec![], 2, 500.0), "qwen3.5:2b"),
            (machine(4.0, vec![], 4, 500.0), "qwen3.5:0.8b"),
        ];
        for (hw, want) in cases {
            // Windows offers only the smallest models: where a larger one would fit, the largest offered.
            let want = if CHAT_MODELS.iter().any(|m| m.tag == want) { want } else { CHAT_MODELS.last().unwrap().tag };
            assert_eq!(recommend(&hw, None).tag, want, "{} GB, {:?}", hw.total_memory_bytes / gb(1.0), hw.gpus);
        }
    }

    #[test]
    fn models_are_listed_smallest_first_and_each_one_is_offered() {
        let sizes: Vec<f64> = ["0.8b", "2b", "4b", "9b", "12b", "35b-a3b"]
            .iter()
            .map(|s| s.trim_end_matches("-a3b").trim_end_matches('b').parse().unwrap())
            .collect();
        let listed: Vec<&str> = ALL_CHAT_MODELS.iter().map(|m| m.tag.split(':').nth(1).unwrap()).collect();
        assert_eq!(listed, ["0.8b", "2b", "4b", "9b", "12b", "35b-a3b"]);
        // This system offers the smallest ones in the same order: all of them, or the first three on Windows.
        let offered: Vec<&str> = CHAT_MODELS.iter().map(|m| m.tag).collect();
        let all: Vec<&str> = ALL_CHAT_MODELS.iter().map(|m| m.tag).collect();
        assert_eq!(offered, all[..if cfg!(windows) { WINDOWS_MODELS } else { all.len() }]);
        assert!(sizes.windows(2).all(|w| w[0] < w[1]));
        // Where Gemma does not fit but Qwen 3.5 9B would not either, the 4B is next.
        let r = recommend(&machine(13.0, apple(), 8, 500.0), None);
        assert_eq!((r.tag, r.budget_bytes < gb(8.5)), ("qwen3.5:4b", true));
    }

    #[test]
    fn a_small_nvidia_card_falls_back_to_cpu_rules() {
        let (budget, accel) = memory_budget(&machine(16.0, nvidia(2.0), 8, 500.0));
        assert_eq!(budget, gb(8.0));
        assert_eq!(accel, Accelerator::Cpu);
    }

    #[test]
    fn disk_space_limits_the_choice() {
        // 8 GB free: the 9B model (its download plus 2 GB headroom) no longer fits.
        let r = recommend(&machine(64.0, apple(), 12, 8.0), None);
        assert_eq!(r.tag, "qwen3.5:4b");
        let r = recommend(&machine(64.0, apple(), 12, 1.0), None);
        assert_eq!(r.tag, "qwen3.5:0.8b");
        assert!(r.reason.contains("Free up disk space"));
    }

    #[test]
    fn a_resumed_download_counts_what_is_on_disk_already() {
        let m = &CHAT_MODELS[2]; // 4.0 GB
        assert!(fits_disk(m, gb(6.0), 0));
        assert!(!fits_disk(m, gb(5.0), 0));
        assert!(fits_disk(m, gb(5.0), gb(1.5)), "1.5 GB of it are downloaded already");
        assert!(fits_disk(m, 0, u64::MAX), "no overflow");
    }

    #[test]
    fn an_edition_limit_caps_the_pick() {
        // An 18 GB Mac runs the 9B model; with a 4B limit it gets 4B.
        assert_eq!(recommend(&machine(18.0, apple(), 11, 500.0), Some("qwen3.5:4b")).tag, "qwen3.5:4b");
        // Below the limit, the hardware still decides.
        assert_eq!(recommend(&machine(8.0, apple(), 8, 500.0), Some("qwen3.5:4b")).tag, "qwen3.5:2b");
    }

    #[test]
    fn detects_this_machine() {
        let hw = detect(&std::env::temp_dir());
        assert!(hw.total_memory_bytes > 0 && hw.cores > 0 && hw.free_disk_bytes > 0, "{hw:?}");
        assert!(CHAT_MODELS.iter().any(|m| m.tag == recommend(&hw, None).tag));
    }
}
