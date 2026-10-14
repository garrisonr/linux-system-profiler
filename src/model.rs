use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    pub schema_version: u32,
    pub tool_version: String,
    pub generated_at: String,
    pub hostname: String,
    pub system: SystemInfo,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub storage: Vec<StorageDevice>,
    pub volumes: Vec<VolumeInfo>,
    pub network: Vec<NetworkInterface>,
    pub graphics: Vec<GpuInfo>,
    pub displays: Vec<DisplayInfo>,
    pub printers: Vec<PrinterInfo>,
    pub bluetooth: Vec<PairedDevice>,
    pub accounts: Vec<UserAccount>,
    pub security: SecurityInfo,
    pub power: Vec<BatteryInfo>,
    pub power_detail: PowerDetail,
    pub pci: Vec<BusDevice>,
    pub usb: Vec<BusDevice>,
    pub audio: Vec<String>,
    pub software: SoftwareInfo,
    pub updates: UpdatesInfo,
    pub health: Vec<HealthFlag>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemInfo {
    pub manufacturer: Option<String>,
    pub product_name: Option<String>,
    pub product_version: Option<String>,
    pub board_vendor: Option<String>,
    pub board_name: Option<String>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
    pub chassis_type: Option<String>,
    pub os_name: Option<String>,
    pub os_id: Option<String>,
    pub os_version_id: Option<String>,
    pub kernel: Option<String>,
    pub arch: Option<String>,
    pub uptime_seconds: Option<u64>,
    pub secure_boot: Option<String>,
    pub session_type: Option<String>,
    pub desktop: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuInfo {
    pub model_name: Option<String>,
    pub vendor: Option<String>,
    pub sockets: Option<u32>,
    pub cores_physical: Option<u32>,
    pub threads_logical: Option<u32>,
    pub max_mhz: Option<f64>,
    pub cur_mhz: Option<f64>,
    pub cache_l3: Option<String>,
    pub flags_sample: Vec<String>,
    pub virtualization: Option<String>,
    pub temp_c: Option<f64>,
    pub tjmax_c: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryInfo {
    pub total_kb: Option<u64>,
    pub available_kb: Option<u64>,
    pub swap_total_kb: Option<u64>,
    pub modules: Vec<MemoryModule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryModule {
    pub locator: String,
    pub size: Option<String>,
    pub mem_type: Option<String>,
    pub speed: Option<String>,
    pub manufacturer: Option<String>,
    pub part_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageDevice {
    pub name: String,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub size_bytes: Option<u64>,
    pub rotational: Option<bool>,
    pub tran: Option<String>,
    pub link: Option<String>,
    pub mountpoints: Vec<String>,
    pub fstype: Option<String>,
    pub smart_health: Option<String>,
    pub smart_temp_c: Option<i64>,
    pub power_on_hours: Option<i64>,
    pub percentage_used: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkInterface {
    pub name: String,
    pub kind: String,
    pub is_virtual: bool,
    pub mac: Option<String>,
    pub operstate: Option<String>,
    pub speed_mbps: Option<i64>,
    pub driver: Option<String>,
    pub addresses: Vec<String>,
    pub wifi: Option<WifiInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WifiInfo {
    pub phy_modes: Option<String>,
    pub signal_dbm: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuInfo {
    pub card: String,
    pub vendor_device: Option<String>,
    pub driver_in_use: Option<String>,
    pub vram_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatteryInfo {
    pub name: String,
    pub status: Option<String>,
    pub capacity_percent: Option<i64>,
    pub charge_full: Option<i64>,
    pub charge_full_design: Option<i64>,
    pub health_percent: Option<f64>,
    pub cycle_count: Option<i64>,
    pub manufacturer: Option<String>,
    pub model_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BusDevice {
    pub id: String,
    pub description: String,
    pub driver: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayInfo {
    pub connector: String,
    pub name: Option<String>,
    pub manufacturer: Option<String>,
    pub serial: Option<String>,
    pub width_cm: Option<u32>,
    pub height_cm: Option<u32>,
    pub manufacture_year: Option<u32>,
    pub resolution: Option<String>,
    /// Refresh rate (Hz) of the panel's preferred EDID timing (v0.6).
    #[serde(default)]
    pub refresh_hz: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpsInfo {
    pub model: Option<String>,
    pub state: Option<String>,
    pub percentage: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PowerDetail {
    pub ac_online: Option<bool>,
    pub governor: Option<String>,
    pub epp: Option<String>,
    pub ups: Option<UpsInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerStatus {
    /// running | not-running | inaccessible | not-installed
    pub state: String,
    pub containers: u32,
    pub containers_running: u32,
    pub images: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VolumeInfo {
    pub mount: String,
    pub fs_type: String,
    pub device: Option<String>,
    pub subvolume: Option<String>,
    pub size_bytes: Option<u64>,
    pub used_bytes: Option<u64>,
    pub avail_bytes: Option<u64>,
    pub use_percent: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrinterInfo {
    pub name: String,
    pub state: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PairedDevice {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserAccount {
    pub name: String,
    pub uid: u32,
    pub shell: Option<String>,
    pub home: Option<String>,
    pub admin: bool,
    pub last_login: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityInfo {
    pub firewall_active: Option<bool>,
    pub firewall_service: Option<String>,
    pub luks_volumes: usize,
    pub sshd_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NamedVersion {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapInfo {
    pub name: String,
    pub version: String,
    pub publisher: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DockerImage {
    pub repository_tag: String,
    pub size: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SoftwareInfo {
    pub glibc: Option<String>,
    pub gcc_runtime: Option<String>,
    pub loaded_modules_count: Option<usize>,
    pub loaded_modules_sample: Vec<String>,
    pub packages_count: Option<usize>,
    pub package_manager: Option<String>,
    pub flatpaks_count: Option<usize>,
    pub flatpaks_list: Vec<NamedVersion>,
    pub running_services_count: Option<usize>,
    pub explicit_packages: Vec<NamedVersion>,
    pub last_upgrade: Option<String>,
    pub snaps: Vec<SnapInfo>,
    pub docker_images: Vec<DockerImage>,
    pub docker_status: Option<DockerStatus>,
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdatesInfo {
    pub checked: bool,
    pub system_updates: Vec<UpdateItem>,
    pub firmware_updates: Vec<UpdateItem>,
    pub flatpak_updates: Vec<UpdateItem>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateItem {
    pub name: String,
    pub current: Option<String>,
    pub available: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthFlag {
    pub severity: String, // green | amber | red | info
    pub component: String,
    pub message: String,
    pub hint: Option<String>,
}
