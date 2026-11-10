use super::*;
const TREE_LIMIT: usize = 8 * 1024 * 1024;
pub(super) fn read_tree(path: &Path) -> Result<Option<Tree>> {
    crate::platform::secure_storage::safe(path)?;
    if !path.exists() {
        return Ok(None);
    }
    if !path.is_dir() {
        bail!("同名路径不是 Skill 文件夹");
    }
    fn walk(
        base: &Path,
        path: &Path,
        out: &mut Tree,
        total: &mut usize,
        depth: usize,
    ) -> Result<()> {
        if depth > 16 {
            bail!("Skill 文件夹层级过深");
        }
        crate::platform::secure_storage::safe(path)?;
        for child in fs::read_dir(path)? {
            let path = child?.path();
            crate::platform::secure_storage::safe(&path)?;
            if path.is_dir() {
                walk(base, &path, out, total, depth + 1)?;
            } else if path.is_file() {
                let size = fs::metadata(&path)?.len();
                if size > TREE_LIMIT as u64
                    || *total + size as usize > TREE_LIMIT
                    || out.len() >= 512
                {
                    bail!("单个 Skill 限制为 8 MiB、512 个文件");
                }
                let bytes = read_bytes(&path)?.context("Skill 文件在读取期间变化")?;
                *total += bytes.len();
                if *total > TREE_LIMIT {
                    bail!("Skill 超过 8 MiB");
                }
                let name = path
                    .strip_prefix(base)?
                    .to_str()
                    .context("Skill 路径必须为 Unicode")?
                    .replace('\\', "/");
                out.insert(name, STANDARD.encode(bytes));
            } else {
                bail!("Skill 含特殊文件，未导入");
            }
        }
        Ok(())
    }
    let mut out = Tree::new();
    walk(path, path, &mut out, &mut 0, 0)?;
    Ok(Some(out))
}
pub(super) fn tree_hash(tree: &Tree) -> String {
    hash(&serde_json::to_vec(tree).unwrap_or_default())
}
fn write_tree(path: &Path, tree: &Tree) -> Result<()> {
    if path.exists() {
        bail!("Skill 暂存目录已存在");
    }
    crate::platform::secure_storage::safe(path)?;
    fs::create_dir_all(path)?;
    crate::platform::secure_storage::private_directory(path)?;
    for (name, bytes) in tree {
        if name.contains('\\')
            || name
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == ".." || p.contains(':'))
            || Path::new(name).is_absolute()
        {
            bail!("Skill 文件路径无效");
        }
        crate::platform::secure_storage::write_auth(&path.join(name), &STANDARD.decode(bytes)?)?;
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    target: String,
    name: String,
    nonce: String,
    path: String,
    before: Option<Tree>,
    after: Option<Tree>,
}
fn paths(p: &Pending, targets: &[Target]) -> Result<(PathBuf, PathBuf, PathBuf)> {
    valid_name(&p.name)?;
    uuid::Uuid::parse_str(&p.nonce)?;
    let target = targets
        .iter()
        .find(|t| t.id == p.target)
        .context("Skill 事务工具无效")?;
    let path = target.skills.join(&p.name);
    if path.to_string_lossy() != p.path {
        bail!("Skill 同步尚未完成且配置目录已变化，请恢复原配置目录后重试");
    }
    let stage = target.skills.join(format!(".yunkuo-{}.stage", p.nonce));
    let previous = target.skills.join(format!(".yunkuo-{}.previous", p.nonce));
    for path in [&path, &stage, &previous] {
        crate::platform::secure_storage::safe(path)?;
    }
    Ok((path, stage, previous))
}
fn remove_stage(path: &Path, expected: &Option<Tree>) -> Result<()> {
    let Some(actual) = read_tree(path)? else {
        return Ok(());
    };
    if !expected
        .as_ref()
        .is_some_and(|expected| actual.iter().all(|(k, v)| expected.get(k) == Some(v)))
    {
        bail!("Skill 暂存文件发生变化，副本已保留");
    }
    fs::remove_dir_all(path)?;
    Ok(())
}
pub(super) fn recover(root: &Path, targets: &[Target]) -> Result<()> {
    let journal = root.join("skill-pending.coc");
    let Some(p): Option<Pending> = read_secret(&journal)? else {
        return Ok(());
    };
    let (path, stage, previous) = paths(&p, targets)?;
    let current = read_tree(&path)?;
    if current == p.after {
        remove_stage(&previous, &p.before)?;
    } else if current == p.before && !previous.exists() {
        // 尚未发布；丢弃有完整哈希校验的暂存副本。
    } else if current.is_none() && read_tree(&previous)? == p.before && p.before.is_some() {
        fs::rename(&previous, &path)?;
    } else {
        bail!("Skill 同步中断后目标被修改，已保留恢复副本，请勿覆盖同名文件夹");
    }
    remove_stage(&stage, &p.after)?;
    fs::remove_file(journal)?;
    Ok(())
}
pub(super) fn sync(root: &Path, entry: &Entry, target: &Target, force: bool) -> Result<()> {
    let path = target.skills.join(&entry.name);
    let before = read_tree(&path)?;
    let after = (entry.enabled.get(&target.id) == Some(&true)).then(|| entry.files.clone());
    if before != after
        && before.is_some()
        && !force
        && !entry.applied.get(&target.id).is_some_and(|receipt| {
            before
                .as_ref()
                .is_some_and(|tree| tree_hash(tree) == receipt.hash)
        })
    {
        return Err(anyhow::Error::new(Conflict(
            "Skill 在同步前被外部修改，原文件夹保留".into(),
        )));
    }
    let p = Pending {
        target: target.id.clone(),
        name: entry.name.clone(),
        nonce: uuid::Uuid::new_v4().to_string(),
        path: path.to_string_lossy().into(),
        before,
        after,
    };
    let (path, stage, previous) = paths(&p, std::slice::from_ref(target))?;
    let backup = root
        .join("backups")
        .join(format!("skill-{}-{}.coc", entry.id, target.id));
    write_secret(&backup, &p)?;
    write_secret(&root.join("skill-pending.coc"), &p)?;
    let result = (|| {
        if let Some(after) = &p.after {
            write_tree(&stage, after)?;
        }
        if read_tree(&path)? != p.before {
            bail!("Skill 在同步期间变化，未覆盖");
        }
        if path.exists() {
            fs::rename(&path, &previous)?;
        }
        if p.after.is_some() {
            fs::rename(&stage, &path)?;
        }
        Ok(())
    })();
    // 同步失败时优先回退原目录；下一次打开能力库仍可继续完成恢复。
    recover(root, std::slice::from_ref(target))?;
    result
}
#[cfg(test)]
pub(super) fn test_recovery() {
    let f = crate::test_support::FixtureDir::new();
    let root = f.path().join("app/capability-library");
    let targets = targets(&f.path().join("user"), false, &[]).unwrap();
    let target = &targets[0];
    let before = Tree::from([("SKILL.md".into(), STANDARD.encode("before"))]);
    let after = Tree::from([
        ("SKILL.md".into(), STANDARD.encode("after")),
        ("scripts/run.txt".into(), STANDARD.encode("fixture")),
    ]);
    let p = Pending {
        target: target.id.clone(),
        name: "recover-test".into(),
        nonce: uuid::Uuid::new_v4().to_string(),
        path: target.skills.join("recover-test").to_string_lossy().into(),
        before: Some(before.clone()),
        after: Some(after.clone()),
    };
    let (path, stage, previous) = paths(&p, &targets).unwrap();
    write_tree(&previous, &before).unwrap();
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("SKILL.md"), "after").unwrap();
    write_secret(&root.join("skill-pending.coc"), &p).unwrap();
    recover(&root, &targets).unwrap();
    assert_eq!(read_tree(&path).unwrap(), Some(before));
    assert!(!stage.exists());
    assert!(!previous.exists());
    fs::rename(&path, &previous).unwrap();
    write_tree(&path, &after).unwrap();
    write_secret(&root.join("skill-pending.coc"), &p).unwrap();
    recover(&root, &targets).unwrap();
    assert_eq!(read_tree(&path).unwrap(), Some(after));
    assert!(!previous.exists());
}
