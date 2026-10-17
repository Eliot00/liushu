//! 读 Rime 码表（`.dict.yaml`）成「(编码, 候选)」流。
//!
//! Rime 的码表文件是「YAML 头 + 制表符数据」：`---` 与 `...` 之间是表头，`...` 之后一行一条
//! `字段<TAB>字段`，字段的含义由表头里的 `columns` 决定（不写就是 `text code weight`）。
//! 表头里的 `import_tables` 会按名字引入同目录下的 `<名字>.dict.yaml`——山人方案的
//! `shanren_sc.dict.yaml` 就是这么把「一简表 + 字表 + 词表」拼成一本的。
//!
//! 这里只做建库需要的事，不引 YAML 库：认得 `columns` / `import_tables` 两个块，其余表头行
//! （`name` / `version` / `sort` …）跳过。数据行里字段多出来的忽略，少到连 `text` 都没有就报错。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use liushu_core::dict::DictItem;
use liushu_core::error::LiushuError;

/// 一行解析出来的候选。
pub type Row = (Box<str>, DictItem);

/// 「字段名 → 第几列」。
type Columns = Vec<String>;

/// 解析结果：数据行 + 引入的子表。
struct Table {
    rows: Vec<Row>,
    imports: Vec<String>,
}

/// 读一本码表（含它 `import_tables` 引入的子表），按「子表在前、自己在后」的顺序返回。
pub fn read(path: impl AsRef<Path>) -> Result<Vec<Row>, LiushuError> {
    let mut rows = Vec::new();
    let mut visited = HashSet::new();
    read_into(path.as_ref(), &mut rows, &mut visited)?;
    Ok(rows)
}

/// 递归读，`visited` 防 `import_tables` 成环。
fn read_into(
    path: &Path,
    rows: &mut Vec<Row>,
    visited: &mut HashSet<PathBuf>,
) -> Result<(), LiushuError> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(key) {
        return Ok(());
    }

    let body = std::fs::read_to_string(path)?;
    let table = parse(path, &body)?;

    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    for name in &table.imports {
        // 子表名是相对当前文件所在目录的 `<name>.dict.yaml`。
        let child = directory.join(format!("{name}.dict.yaml"));
        if !child.exists() {
            return Err(LiushuError::Other(format!(
                "{}: import_tables 里的 {name} 找不到（{}）",
                path.display(),
                child.display()
            )));
        }
        read_into(&child, rows, visited)?;
    }
    rows.extend(table.rows);
    Ok(())
}

/// 解析单个文件的表头与数据行。
fn parse(path: &Path, body: &str) -> Result<Table, LiushuError> {
    let mut in_header = false;
    let mut header_done = false;
    let mut import_block = false;
    let mut column_block = false;
    let mut columns: Columns = Vec::new();
    let mut imports: Vec<String> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut skipped_empty_text = 0usize;
    let mut clamped_weight = 0usize;

    for (number, raw) in body.lines().enumerate() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();

        if header_done {
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            let value = |name: &str| -> Option<&str> {
                let index = columns.iter().position(|column| column == name)?;
                fields.get(index).copied()
            };
            // text / code 是必需的列：整列都缺（而不是这一行少写了 weight）说明数据行比表头短。
            for required in ["text", "code"] {
                let index = columns
                    .iter()
                    .position(|column| column == required)
                    .expect("parse 时已经检查过 text / code 列");
                if fields.len() <= index {
                    let hint = if required == "code" {
                        "（Rime 的 `encoder/rules` 会把只有词的条目自动编码，这一步我们没实现：这条要么补上编码，要么从这个 import_tables 里去掉）"
                    } else {
                        ""
                    };
                    return Err(LiushuError::Other(format!(
                        "{}: 第 {} 行只有 {} 列，缺 `{required}` 列{hint}",
                        path.display(),
                        number + 1,
                        fields.len()
                    )));
                }
            }
            let text = value("text").unwrap_or_default();
            let code = value("code").unwrap_or_default();
            // 空文本的行是部件/占位行（`shanren_sc.words` 开头一堆），进了候选列表就是个
            // 空字符串，直接不要。空编码同理：没有编码就查不到。
            if text.is_empty() || code.is_empty() {
                skipped_empty_text += 1;
                continue;
            }
            let (weight, clamped) = parse_weight(value("weight"));
            clamped_weight += usize::from(clamped);
            rows.push((
                code.into(),
                DictItem {
                    text: text.into(),
                    weight,
                    comment: None,
                },
            ));
            continue;
        }

        if trimmed == "---" {
            in_header = true;
            continue;
        }
        if trimmed == "..." {
            if columns.is_empty() {
                // Rime 的默认列顺序。
                columns = ["text", "code", "weight"]
                    .iter()
                    .map(|column| (*column).to_owned())
                    .collect();
            }
            if !columns.iter().any(|column| column == "text")
                || !columns.iter().any(|column| column == "code")
            {
                return Err(LiushuError::Other(format!(
                    "{}: columns 里必须有 text 与 code（现在 {columns:?}）",
                    path.display()
                )));
            }
            header_done = true;
            in_header = false;
            continue;
        }
        if !in_header {
            continue;
        }

        // 表头：只关心两个块列表，其余键值跳过。
        if let Some(rest) = trimmed.strip_prefix("import_tables:") {
            import_block = true;
            column_block = false;
            // `import_tables: [a, b]` 这种行内写法也认。
            let inline = rest.trim().trim_start_matches('[').trim_end_matches(']');
            if !inline.is_empty() {
                for name in inline.split(',') {
                    let name = name.trim().trim_matches(|c| c == '"' || c == '\'');
                    if !name.is_empty() {
                        imports.push(name.to_owned());
                    }
                }
                import_block = false;
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("columns:") {
            column_block = true;
            import_block = false;
            let inline = rest.trim().trim_start_matches('[').trim_end_matches(']');
            if !inline.is_empty() {
                for name in inline.split(',') {
                    columns.push(name.trim().to_owned());
                }
                column_block = false;
            }
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            let item = item.trim().trim_matches(|c| c == '"' || c == '\'');
            if import_block {
                imports.push(item.to_owned());
            } else if column_block {
                columns.push(item.to_owned());
            }
            continue;
        }
        // 其它表头键：块列表到此为止。
        if !trimmed.starts_with('#') {
            import_block = false;
            column_block = false;
        }
    }

    if skipped_empty_text > 0 || clamped_weight > 0 {
        eprintln!(
            "{}: 跳过 {skipped_empty_text} 行（空文本或空编码），{clamped_weight} 行权重超出 u32 被夹住",
            path.display()
        );
    }

    Ok(Table { rows, imports })
}

/// Rime 的权重可以是整数、浮点，也可以没有。浮点四舍五入，正数不要被舍成 0。
fn parse_weight(field: Option<&str>) -> (u32, bool) {
    let Some(field) = field.map(str::trim).filter(|field| !field.is_empty()) else {
        return (0, false);
    };
    let Ok(value) = field.parse::<f64>() else {
        return (0, false);
    };
    if !value.is_finite() || value <= 0.0 {
        return (0, false);
    }
    if value >= f64::from(u32::MAX) {
        return (u32::MAX, true);
    }
    let rounded = value.round().max(1.0);
    (rounded as u32, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("liushu-rime-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_row_shorter_than_its_required_columns_is_an_error_with_its_line_number() {
        let dir = temp("short");
        let path = dir.join("demo.dict.yaml");
        std::fs::write(&path, "---\n...\n你\twqiy\t1\n只有文本\n").unwrap();
        let error = read(&path).unwrap_err().to_string();
        assert!(error.contains("第 4 行"), "{error}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_a_plain_table_with_the_default_columns() {
        let dir = temp("plain");
        let path = dir.join("demo.dict.yaml");
        std::fs::write(
            &path,
            "---\nname: demo\nsort: by_weight\n...\n你\twqiy\t100\n好\tvbg\t70\n空\t\t5\n",
        )
        .unwrap();
        let rows = read(&path).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0.as_ref(), "wqiy");
        assert_eq!(rows[0].1.text.as_ref(), "你");
        assert_eq!(rows[0].1.weight, 100);
        assert_eq!(rows[1].0.as_ref(), "vbg");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn honours_a_columns_header() {
        let dir = temp("columns");
        let path = dir.join("demo.dict.yaml");
        // 列顺序反着来，还带一列不认识的。
        std::fs::write(
            &path,
            "---\ncolumns:\n  - code\n  - text\n  - extra\n...\nwqiy\t你\tx\n",
        )
        .unwrap();
        let rows = read(&path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0.as_ref(), "wqiy");
        assert_eq!(rows[0].1.text.as_ref(), "你");
        // 没有 weight 列就是 0。
        assert_eq!(rows[0].1.weight, 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn follows_import_tables_relative_to_the_file() {
        let dir = temp("imports");
        std::fs::write(dir.join("child.dict.yaml"), "---\n...\n子\tzi\t5\n").unwrap();
        let path = dir.join("parent.dict.yaml");
        std::fs::write(
            &path,
            "---\ncolumns:\n  - text\n  - code\n  - weight\nimport_tables:\n  - child\n...\n父\tfu\t9\n",
        )
        .unwrap();
        let rows = read(&path).unwrap();
        // 子表在前，自己在前。
        assert_eq!(
            rows.iter()
                .map(|(code, item)| (item.text.as_ref(), code.as_ref(), item.weight))
                .collect::<Vec<_>>(),
            [("子", "zi", 5), ("父", "fu", 9)]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_import_is_an_error_not_a_silent_skip() {
        let dir = temp("missing");
        let path = dir.join("parent.dict.yaml");
        std::fs::write(&path, "---\nimport_tables:\n  - nope\n...\n父\tfu\t9\n").unwrap();
        assert!(read(&path).unwrap_err().to_string().contains("nope"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn import_cycles_do_not_hang() {
        let dir = temp("cycle");
        std::fs::write(
            dir.join("a.dict.yaml"),
            "---\nimport_tables:\n  - b\n...\n甲\tj\t1\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("b.dict.yaml"),
            "---\nimport_tables:\n  - a\n...\n乙\ty\t2\n",
        )
        .unwrap();
        let rows = read(dir.join("a.dict.yaml")).unwrap();
        assert_eq!(rows.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn weights_accept_floats_and_never_round_a_positive_down_to_zero() {
        assert_eq!(parse_weight(Some("100")), (100, false));
        assert_eq!(parse_weight(Some("1.4")), (1, false));
        assert_eq!(parse_weight(Some("0.4")), (1, false));
        assert_eq!(parse_weight(Some("0")), (0, false));
        assert_eq!(parse_weight(Some("")), (0, false));
        assert_eq!(parse_weight(None), (0, false));
        assert_eq!(parse_weight(Some("99999999999")), (u32::MAX, true));
    }

    #[test]
    fn rows_without_text_or_code_are_skipped() {
        let dir = temp("empty");
        let path = dir.join("demo.dict.yaml");
        std::fs::write(&path, "---\n...\n宀\ta\t0\n\ta\t0\n\t\t\n你\twqiy\t1\n").unwrap();
        let rows = read(&path).unwrap();
        // 部件行（文本是部首）留着，空文本 / 空编码的行扔掉。
        assert_eq!(
            rows.iter()
                .map(|(_, item)| item.text.as_ref())
                .collect::<Vec<_>>(),
            ["宀", "你"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
