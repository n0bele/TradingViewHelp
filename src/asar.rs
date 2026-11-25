//! 最小化的 asar 读取器：把 Electron 的 app.asar 解包成普通目录。
//!
//! asar 布局（全部小端）：
//! ```text
//! 0  u32 = 4
//! 4  u32  header_size          // 指向数据区起点的相对偏移
//! 8  u32  header_string_size   // 对齐填充后的 JSON 长度
//! 12 u32  header_object_size   // 真实 JSON 长度
//! 16 ..   JSON
//! 8 + header_size ..  文件数据
//! ```

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct ExtractStats {
    pub files: usize,
    pub skipped_links: usize,
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub fn extract(asar: &Path, out: &Path) -> Result<ExtractStats> {
    let mut f = File::open(asar).with_context(|| format!("打开 {}", asar.display()))?;
    let total = f.metadata()?.len();
    let mut head = [0u8; 16];
    f.read_exact(&mut head).context("读取 asar 头失败")?;
    if u32le(&head, 0) != 4 {
        bail!("不是 asar 归档（头部标记不符）");
    }
    let header_size = u32le(&head, 4) as u64;
    let obj_size = u32le(&head, 12) as u64;
    let data_start = 8 + header_size;
    if data_start > total || obj_size == 0 || obj_size > 32 * 1024 * 1024 {
        bail!("asar 头部损坏（data_start={data_start}, obj_size={obj_size}, len={total}）");
    }

    let mut buf = vec![0u8; obj_size as usize];
    f.seek(SeekFrom::Start(16))?;
    f.read_exact(&mut buf)?;
    let header: Value = serde_json::from_slice(&buf).context("asar 头部 JSON 解析失败")?;

    let unpacked = PathBuf::from(format!("{}.unpacked", asar.to_string_lossy()));
    fs::create_dir_all(out)?;

    let mut st = ExtractStats::default();
    walk(&mut f, &header, out, Path::new(""), data_start, &unpacked, &mut st)?;
    Ok(st)
}

fn walk(
    f: &mut File,
    node: &Value,
    out: &Path,
    rel: &Path,
    base: u64,
    unpacked: &Path,
    st: &mut ExtractStats,
) -> Result<()> {
    let Some(map) = node.get("files").and_then(|v| v.as_object()) else {
        return Ok(());
    };
    for (name, child) in map {
        // 只接受单层普通文件名，挡掉路径穿越与绝对路径
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
            || name.contains(':')
        {
            bail!("asar 中出现非法路径名 {name:?}");
        }
        let crel = rel.join(name);

        if child.get("files").is_some() {
            fs::create_dir_all(out.join(&crel))?;
            walk(f, child, out, &crel, base, unpacked, st)?;
            continue;
        }
        if child.get("link").is_some() {
            // 该应用没有符号链接条目，遇到就跳过，不静默猜测
            st.skipped_links += 1;
            continue;
        }

        let size = child.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let is_unpacked = child
            .get("unpacked")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dest = out.join(&crel);
        if let Some(p) = dest.parent() {
            fs::create_dir_all(p)?;
        }

        if is_unpacked {
            fs::copy(unpacked.join(&crel), &dest)
                .with_context(|| format!("复制 unpacked 文件 {}", crel.display()))?;
        } else {
            let off: u64 = child
                .get("offset")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            f.seek(SeekFrom::Start(base + off))?;
            let mut data = vec![0u8; size as usize];
            f.read_exact(&mut data)
                .with_context(|| format!("读取 {} 数据失败", crel.display()))?;
            let mut o = File::create(&dest)?;
            o.write_all(&data)?;
        }
        st.files += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
