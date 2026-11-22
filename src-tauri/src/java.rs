//! Java: выбор версии под сервер и установка Eclipse Temurin из Adoptium API.
//!
//! JRE лежат в `%LOCALAPPDATA%\Hearth\java\<major>\`, системная Java не нужна.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{Error, Result};
use crate::fsx;
use crate::net::{Checksum, Http};

const SERVICE: &str = "Adoptium";
const META_FILE: &str = "hearth-java.json";

/// LTS-версии Temurin, на которые можно перейти, если точной версии нет
/// (например, Java 16 для 1.17 больше не публикуется, берём 17).
const LTS: [u32; 5] = [8, 11, 17, 21, 25];

/// Запасная таблица на случай, когда ни API ядра, ни Mojang не сказали, какая Java нужна.
pub fn fallback_major(game: &str) -> u32 {
    let base = game.split(['-', ' ']).next().unwrap_or(game);
    let parts: Vec<u32> = base.split('.').map(|p| p.parse().unwrap_or(0)).collect();
    match parts.as_slice() {
        // Новая нумерация Mojang: 26.1, 26.2...
        [major, ..] if *major >= 26 => 25,
        [1, minor, rest @ ..] => {
            let patch = rest.first().copied().unwrap_or(0);
            match (*minor, patch) {
                (m, _) if m >= 21 => 21,
                (20, p) if p >= 5 => 21,
                (m, _) if m >= 18 => 17,
                (17, _) => 16,
                _ => 8,
            }
        }
        // Снапшоты старого формата (24w14a) и всё непонятное.
        _ => 21,
    }
}

/// Порядок перебора: сначала ровно нужная версия, потом более новые LTS.
pub fn candidates(required: u32) -> Vec<u32> {
    let mut list = vec![required];
    list.extend(LTS.iter().copied().filter(|v| *v > required));
    list
}

/// Флаги JVM для ядер, которые не дают своих рекомендаций (Vanilla, Fabric).
/// Это флаги Aikar в том виде, в каком их отдаёт API Paper для соответствующей Java.
pub fn default_jvm_flags(java_major: u32) -> Vec<String> {
    let mut flags: Vec<&str> = vec![
        "-XX:+AlwaysPreTouch",
        "-XX:+DisableExplicitGC",
        "-XX:+PerfDisableSharedMem",
        "-XX:+UnlockExperimentalVMOptions",
        "-XX:+UseG1GC",
        "-XX:G1HeapRegionSize=8M",
        "-XX:G1MaxNewSizePercent=40",
        "-XX:G1MixedGCCountTarget=4",
        "-XX:G1MixedGCLiveThresholdPercent=90",
        "-XX:G1NewSizePercent=30",
        "-XX:G1RSetUpdatingPauseTimePercent=5",
        "-XX:G1ReservePercent=20",
        "-XX:InitiatingHeapOccupancyPercent=15",
        "-XX:MaxTenuringThreshold=1",
        "-XX:SurvivorRatio=32",
    ];
    if java_major < 25 {
        flags.extend([
            "-XX:+ParallelRefProcEnabled",
            "-XX:G1HeapWastePercent=5",
            "-XX:MaxGCPauseMillis=200",
        ]);
    }
    flags.into_iter().map(String::from).collect()
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InstalledJava {
    pub major: u32,
    /// Например `jdk-21.0.13.1+2`.
    pub release: String,
    /// `jre` или `jdk`.
    pub image: String,
}

#[derive(Deserialize, Debug)]
struct Asset {
    binary: Binary,
    release_name: String,
}

#[derive(Deserialize, Debug)]
struct Binary {
    image_type: String,
    package: Package,
}

#[derive(Deserialize, Debug)]
struct Package {
    name: String,
    link: String,
    checksum: Option<String>,
}

pub struct JavaManager {
    root: PathBuf,
    downloads: PathBuf,
    http: Http,
    install_lock: tokio::sync::Mutex<()>,
}

impl JavaManager {
    pub fn new(root: PathBuf, cache_dir: &Path, http: Http) -> Self {
        Self {
            root,
            downloads: cache_dir.join("downloads"),
            http,
            install_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn exe(&self, major: u32) -> PathBuf {
        let name = if cfg!(windows) { "java.exe" } else { "java" };
        self.root.join(major.to_string()).join("bin").join(name)
    }

    pub fn is_installed(&self, major: u32) -> bool {
        self.exe(major).is_file() && self.root.join(major.to_string()).join(META_FILE).is_file()
    }

    pub fn installed(&self) -> Vec<InstalledJava> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut list: Vec<InstalledJava> = entries
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join(META_FILE)).ok())
            .filter_map(|text| serde_json::from_str::<InstalledJava>(&text).ok())
            .filter(|j| self.exe(j.major).is_file())
            .collect();
        list.sort_by_key(|j| j.major);
        list
    }

    /// Гарантирует, что подходящая Java установлена, и возвращает её версию и путь к java.exe.
    pub async fn ensure(
        &self,
        required: u32,
        mut progress: impl FnMut(u64, Option<u64>) + Send,
    ) -> Result<(u32, PathBuf)> {
        let _guard = self.install_lock.lock().await;
        for major in candidates(required) {
            if self.is_installed(major) {
                return Ok((major, self.exe(major)));
            }
            if let Some(asset) = self.find_asset(major).await? {
                self.install(major, asset, &mut progress).await?;
                return Ok((major, self.exe(major)));
            }
            tracing::info!("Temurin {major} для Windows не найден, пробую более новую LTS");
        }
        Err(Error::JavaUnavailable(required))
    }

    async fn find_asset(&self, major: u32) -> Result<Option<Asset>> {
        let arch = match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            "x86" => "x32",
            _ => "x64",
        };
        for image in ["jre", "jdk"] {
            let url = format!(
                "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={arch}&image_type={image}&os=windows&vendor=eclipse"
            );
            let assets: Vec<Asset> = match self.http.get_json(SERVICE, &url).await {
                Ok(a) => a,
                Err(e) if e.is_not_found() => Vec::new(),
                Err(e) => return Err(e),
            };
            if let Some(asset) = assets
                .into_iter()
                .find(|a| a.binary.package.name.ends_with(".zip"))
            {
                return Ok(Some(asset));
            }
        }
        Ok(None)
    }

    async fn install(
        &self,
        major: u32,
        asset: Asset,
        progress: &mut (impl FnMut(u64, Option<u64>) + Send),
    ) -> Result<()> {
        tracing::info!("Устанавливаю Java {major}: {}", asset.release_name);
        let zip_path = self.downloads.join(&asset.binary.package.name);
        let checksum = asset.binary.package.checksum.clone().map(Checksum::Sha256);
        self.http
            .download(&asset.binary.package.link, &zip_path, checksum.as_ref(), |d, t| progress(d, t))
            .await?;

        let root = self.root.clone();
        let meta = InstalledJava {
            major,
            release: asset.release_name.clone(),
            image: asset.binary.image_type.clone(),
        };
        let zip_for_task = zip_path.clone();
        tokio::task::spawn_blocking(move || extract_jre(&zip_for_task, &root, &meta)).await??;
        let _ = tokio::fs::remove_file(&zip_path).await;
        Ok(())
    }
}

/// Распаковывает архив Temurin в `root/<major>`. Внутри архива одна папка
/// вида `jdk-21.0.13.1+2-jre`, её содержимое и становится `root/<major>`.
fn extract_jre(zip_path: &Path, root: &Path, meta: &InstalledJava) -> Result<()> {
    let tmp = root.join(format!(".tmp-{}", meta.major));
    let dest = root.join(meta.major.to_string());
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;

    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    archive.extract(&tmp)?;

    let dirs: Vec<PathBuf> = std::fs::read_dir(&tmp)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    let content = match dirs.as_slice() {
        [single] if single.join("bin").is_dir() => single.clone(),
        _ if tmp.join("bin").is_dir() => tmp.clone(),
        _ => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(Error::other("Неожиданная структура архива Java: не найдена папка bin"));
        }
    };

    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    fsx::rename_with_retry_sync(&content, &dest)?;
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    let meta_json = serde_json::to_vec_pretty(meta).expect("meta serializes");
    std::fs::write(dest.join(META_FILE), meta_json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_table_matches_spec() {
        assert_eq!(fallback_major("26.1"), 25);
        assert_eq!(fallback_major("26.4-snapshot-10"), 25);
        assert_eq!(fallback_major("1.21.11"), 21);
        assert_eq!(fallback_major("1.20.5"), 21);
        assert_eq!(fallback_major("1.20.4"), 17);
        assert_eq!(fallback_major("1.18"), 17);
        assert_eq!(fallback_major("1.17.1"), 16);
        assert_eq!(fallback_major("1.16.5"), 8);
        assert_eq!(fallback_major("1.12.2"), 8);
        assert_eq!(fallback_major("24w14a"), 21);
    }

    #[test]
    fn candidates_go_up_to_newer_lts() {
        assert_eq!(candidates(16), [16, 17, 21, 25]);
        assert_eq!(candidates(21), [21, 25]);
        assert_eq!(candidates(8), [8, 11, 17, 21, 25]);
    }

    #[test]
    fn older_java_gets_full_aikar_flags() {
        assert!(default_jvm_flags(21).contains(&"-XX:MaxGCPauseMillis=200".to_string()));
        assert!(!default_jvm_flags(25).contains(&"-XX:MaxGCPauseMillis=200".to_string()));
    }

    #[test]
    fn parses_adoptium_asset() {
        let json = r#"[{"binary":{"architecture":"x64","image_type":"jre","os":"windows",
          "package":{"checksum":"e15e","link":"https://github.com/x/OpenJDK21U-jre_x64_windows_hotspot_21.0.13.1_2.zip","name":"OpenJDK21U-jre_x64_windows_hotspot_21.0.13.1_2.zip","size":49108427}},
          "release_name":"jdk-21.0.13.1+2","vendor":"eclipse"}]"#;
        let assets: Vec<Asset> = serde_json::from_str(json).unwrap();
        assert_eq!(assets[0].release_name, "jdk-21.0.13.1+2");
        assert!(assets[0].binary.package.name.ends_with(".zip"));
    }
}
