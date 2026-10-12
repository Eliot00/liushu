//! 整句解码：拼音 → 词图上的 k-best 维特比（带束搜索）。
//!
//! 旧的 `hmm` 是字符级的：每个音节查「哪些字读这个音」，字与字之间做维特比——它不知道「词」，
//! 也处理不了同一串拼音的多种切分。现在的做法是词图：
//!
//! 1. [`crate::pinyin`] 给出所有合法音节跨度，`xian` 因此同时有 `xian` 和 `xi` + `an`（还有
//!    `xia` + `n`）几条路。
//! 2. 每条「起始位置 → 结束位置」的跨度去词库查**精确**匹配的编码，命中几个词就是几条边。
//! 3. 边权 = 词库词频的 log（同一个编码下的候选互相竞争）+ 词级语言模型的 `ln P(词 | 前词)`；
//!    模型里没有的词扣一个固定分。
//! 4. 在词图上按位置顺序做 k-best 维特比：每条边保留分数最高的 k 条前驱路径。往同一个节点
//!    攒前 k 条路径是**精确**的——某条前缀要是排到第 k+1 名，它后面接上同样的后缀也一定排在
//!    另外 k 条之后，进不了最终的 k-best。
//!
//! 每个位置的词数按最好一条路径截到 [`BEAM_WIDTH`]（束搜索）：输入越长、同音字越多，词图上
//! 的边就越多，这一步把每键的代价压成常数。

use crate::dict::Dictionary;
use crate::lm::{LanguageModel, dequantize};
use crate::pinyin;

/// 一个整句候选。
#[derive(Debug, Clone, PartialEq)]
pub struct Sentence {
    /// 解出来的文字。
    pub text: String,

    /// 对数概率（自然对数，越大越好）。
    pub score: f64,
}

/// 一个词最多跨几个音节：词库里超过六个音节的词几乎没有，而枚举跨度是立方级的。
const MAX_SPAN_SYLLABLES: usize = 6;

/// 一个跨度最多取几个候选词。同音词在词库里已按词频降序排好，取前几名——「一个音节对应上百个
/// 同音字」是常态，全都进词图会让每个位置的配对数变成平方级，而排在后面的同音词本来就几乎不
/// 可能是答案。
const SPAN_CANDIDATES: usize = 8;

/// 每个位置上最多保留多少个词（按最好的一条路径排序）。
const BEAM_WIDTH: usize = 32;

/// 词库词频换算成 log 概率时的权重。
const DICTIONARY_WEIGHT: f64 = 1.0;

/// 模型里没有这个词时的扣分，`ln(1e-7)` 左右。
const UNKNOWN_WORD: i32 = -258;

/// 一条边的前驱路径。`back == NO_BACK` 表示这条边是句子的第一个词。
#[derive(Debug, Clone, Copy)]
struct Path {
    score: i32,
    back: usize,
    rank: u32,
}

/// 「没有前驱」的哨兵。
const NO_BACK: usize = usize::MAX;

/// 词图上的一条边：`code[from..to]` 这个跨度对应的一个词。
#[derive(Debug)]
struct Edge {
    from: usize,
    to: usize,
    word: String,
    dictionary_score: i32,
}

/// 词频 → 对数概率（Q4 定点）。
///
/// **必须除以词库的总词频**：直接拿 `ln(词频)` 当分数时每一项都是正数，路径上词越多分越高，
/// 解码器会一路切成单音节词。归一之后每一项都是 `ln P(词) ≤ 0`，才是真正的概率。
fn dictionary_score(weight: u32, total_weight: u64) -> i32 {
    let probability = f64::from(weight.max(1)) / total_weight.max(1) as f64;
    (probability.ln() * DICTIONARY_WEIGHT * 16.0).round() as i32
}

/// 把 `code` 解成最多 `k` 个整句候选，按分数从高到低。
///
/// `code` 是拼音字母，怎么切成音节由音节表和词库一起决定；空格与 `'` 只是书写分隔（`ni hao`
/// / `ni'hao` 与 `nihao` 等价），解析前先丢掉。切不动（有拼不出来的字母）或者词图里没有能
/// 走通的路时返回空列表。
pub fn decode(
    dictionary: &Dictionary,
    model: &LanguageModel,
    code: &str,
    k: usize,
) -> Vec<Sentence> {
    let code: String = code
        .chars()
        .filter(|letter| !letter.is_whitespace() && *letter != '\'')
        .flat_map(char::to_lowercase)
        .collect();
    if code.is_empty() || k == 0 {
        return Vec::new();
    }
    let starts = pinyin::positions(&code);
    if !starts.contains(&code.len()) {
        // 有字母拼不出音节，整句无从谈起（候选列表那边仍然会按前缀给候选）。
        return Vec::new();
    }

    // 建图：每个可达的起点，枚举 1..=MAX_SPAN_SYLLABLES 个音节能到的终点，查词库精确匹配。
    let total_weight = dictionary.total_weight();
    let mut edges: Vec<Edge> = Vec::new();
    for &from in &starts {
        for to in pinyin::reachable(&code, from, MAX_SPAN_SYLLABLES) {
            let Some(index) = dictionary.exact(&code[from..to]) else {
                continue;
            };
            for slot in dictionary.slots_at(index).iter().take(SPAN_CANDIDATES) {
                edges.push(Edge {
                    from,
                    to,
                    word: dictionary.slot_text(slot).to_owned(),
                    dictionary_score: dictionary_score(slot.weight(), total_weight),
                });
            }
        }
    }
    if edges.is_empty() {
        return Vec::new();
    }

    // 位置 → 在这里结束的边。
    let mut ending_at: Vec<Vec<usize>> = vec![Vec::new(); code.len() + 1];
    for (index, edge) in edges.iter().enumerate() {
        ending_at[edge.to].push(index);
    }

    // 按位置从小到大推进：一条边的所有前驱都结束在更小的位置上，所以走到 `to` 时前驱已经算完。
    let mut paths: Vec<Vec<Path>> = vec![Vec::new(); edges.len()];
    for &position in &starts {
        for &edge in &ending_at[position] {
            let mut candidates: Vec<Path> = Vec::new();
            if edges[edge].from == 0 {
                let model_score = model
                    .log_prob(None, &edges[edge].word)
                    .unwrap_or(UNKNOWN_WORD);
                candidates.push(Path {
                    score: edges[edge].dictionary_score.saturating_add(model_score),
                    back: NO_BACK,
                    rank: 0,
                });
            } else {
                for &previous in &ending_at[edges[edge].from] {
                    for (rank, path) in paths[previous].iter().enumerate() {
                        let model_score = model
                            .log_prob(Some(&edges[previous].word), &edges[edge].word)
                            .unwrap_or(UNKNOWN_WORD);
                        candidates.push(Path {
                            score: path
                                .score
                                .saturating_add(model_score)
                                .saturating_add(edges[edge].dictionary_score),
                            back: previous,
                            rank: rank as u32,
                        });
                    }
                }
            }
            // 分数相同时按「前驱边的下标、前驱路径名次」定序，保证同一输入每次结果一样。
            candidates.sort_unstable_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then(a.back.cmp(&b.back))
                    .then(a.rank.cmp(&b.rank))
            });
            candidates.truncate(k);
            paths[edge] = candidates;
        }

        // 束搜索：这个位置的词只留最好的一条路径排前 BEAM_WIDTH 的。被丢掉的边清空路径，
        // 后面的位置引用它就自然什么也拿不到。
        if ending_at[position].len() > BEAM_WIDTH {
            let mut kept: Vec<usize> = ending_at[position].clone();
            kept.sort_unstable_by(|&a, &b| {
                best_score(&paths[b])
                    .cmp(&best_score(&paths[a]))
                    .then(a.cmp(&b))
            });
            for &edge in &kept[BEAM_WIDTH..] {
                paths[edge].clear();
            }
        }
    }

    // 收尾：所有结束在末尾的边，把它们的 k-best 路径汇到一起再取前 k 条。
    let mut finals: Vec<(i32, usize, u32)> = Vec::new();
    for &edge in &ending_at[code.len()] {
        for (rank, path) in paths[edge].iter().enumerate() {
            finals.push((path.score, edge, rank as u32));
        }
    }
    finals.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

    let mut sentences: Vec<Sentence> = Vec::new();
    for &(score, edge, rank) in &finals {
        let text = reconstruct(&edges, &paths, edge, rank);
        // 不同的边可能拼出同一句话（同一个词在不同编码下重复出现），只留分数最高的那条。
        if sentences.iter().any(|existing| existing.text == text) {
            continue;
        }
        sentences.push(Sentence {
            text,
            score: dequantize(score),
        });
        if sentences.len() == k {
            break;
        }
    }
    sentences
}

/// 一条边最好的一条路径分数；一条路径都没有时排到最后。
fn best_score(paths: &[Path]) -> i32 {
    paths.first().map_or(i32::MIN, |path| path.score)
}

/// 顺着回指把词拼起来。
fn reconstruct(edges: &[Edge], paths: &[Vec<Path>], edge: usize, rank: u32) -> String {
    let mut words = vec![edges[edge].word.as_str()];
    let mut edge = edge;
    let mut rank = rank as usize;
    loop {
        let path = paths[edge][rank];
        if path.back == NO_BACK {
            break;
        }
        edge = path.back;
        rank = path.rank as usize;
        words.push(edges[edge].word.as_str());
    }
    words.reverse();
    words.concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dict::DictItem;
    use crate::lm;

    fn item(text: &str, weight: u32) -> DictItem {
        DictItem {
            text: text.into(),
            weight,
            comment: None,
        }
    }

    fn dictionary(rows: &[(&str, &[(&str, u32)])]) -> Dictionary {
        Dictionary::from_items(
            rows.iter()
                .map(|(code, candidates)| {
                    (
                        *code,
                        candidates
                            .iter()
                            .map(|(text, weight)| item(text, *weight))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    /// 训练一个小模型，返回模型与需要清理的临时文件。
    fn model(name: &str, corpus: &str) -> (LanguageModel, Vec<std::path::PathBuf>) {
        let corpus_path =
            std::env::temp_dir().join(format!("liushu-sentence-{name}-{}.tsv", std::process::id()));
        let model_path =
            std::env::temp_dir().join(format!("liushu-sentence-{name}-{}.bin", std::process::id()));
        std::fs::write(&corpus_path, corpus).unwrap();
        lm::train(&corpus_path, &model_path).unwrap();
        let model = LanguageModel::open(&model_path).unwrap();
        (model, vec![corpus_path, model_path])
    }

    fn texts(sentences: &[Sentence]) -> Vec<&str> {
        sentences.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn the_language_model_decides_between_two_segmentations() {
        // 两个词库权重一样：`xian` 是一个词，`xi` + `an` 是两个词，谁赢只看语言模型。
        let dictionary = dictionary(&[
            ("xian", &[("先", 1)]),
            ("xi", &[("西", 1)]),
            ("an", &[("安", 1)]),
        ]);

        let (together, cleanup) = model("together", &"西 安\n".repeat(1000));
        let decoded = decode(&dictionary, &together, "xian", 1);
        assert_eq!(texts(&decoded), ["西安"]);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }

        let (apart, cleanup) = model("apart", &"先\n".repeat(1000));
        let decoded = decode(&dictionary, &apart, "xian", 1);
        assert_eq!(texts(&decoded), ["先"]);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_whole_word_beats_its_syllables_when_it_is_the_common_one() {
        let dictionary = dictionary(&[
            ("nihao", &[("你好", 100)]),
            ("ni", &[("你", 1)]),
            ("hao", &[("好", 1)]),
        ]);
        let (model, cleanup) = model("whole-word", &"你好\n".repeat(100));
        assert_eq!(texts(&decode(&dictionary, &model, "nihao", 1)), ["你好"]);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_word_the_dictionary_lacks_is_assembled_from_syllables() {
        // 词库里没有 `nihao`，只能走 `ni` + `hao` 两条边，拼出来的仍然是一句话。
        let dictionary = dictionary(&[("ni", &[("你", 10)]), ("hao", &[("好", 10)])]);
        let (model, cleanup) = model("assembled", &"你 好\n".repeat(10));
        assert_eq!(texts(&decode(&dictionary, &model, "nihao", 1)), ["你好"]);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn the_k_best_sentences_are_distinct_and_ordered() {
        let dictionary = dictionary(&[("ni", &[("你", 100), ("尼", 50)]), ("hao", &[("好", 100)])]);
        // `你 好` 比 `尼 好` 常见得多。
        let corpus = format!("{}{}", "你 好\n".repeat(100), "尼 好\n".repeat(10));
        let (model, cleanup) = model("k-best", &corpus);
        let decoded = decode(&dictionary, &model, "nihao", 2);
        assert_eq!(texts(&decoded), ["你好", "尼好"]);
        assert!(decoded[0].score > decoded[1].score);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_word_the_model_never_saw_is_still_decodable() {
        let dictionary = dictionary(&[("ni", &[("你", 10)]), ("hao", &[("好", 10)])]);
        // 语料里只有别的词：解码要能靠词库词频走通，而不是因为没有 LM 词条就放弃。
        let (model, cleanup) = model("oov", &"天 地\n".repeat(10));
        assert_eq!(texts(&decode(&dictionary, &model, "nihao", 1)), ["你好"]);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn unsegmentable_or_unknown_input_has_no_sentence() {
        let dictionary = dictionary(&[("ni", &[("你", 10)]), ("hao", &[("好", 10)])]);
        let (model, cleanup) = model("nothing", &"你 好\n".repeat(10));
        // `q` 不是音节，切不动。
        assert!(decode(&dictionary, &model, "nihaoq", 3).is_empty());
        // 音节切得动，但词库里一个词都没有。
        assert!(decode(&dictionary, &model, "zhongguo", 3).is_empty());
        assert!(decode(&dictionary, &model, "", 3).is_empty());
        assert!(decode(&dictionary, &model, "nihao", 0).is_empty());
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn decoding_the_same_input_twice_gives_the_same_answer() {
        let dictionary = dictionary(&[
            ("ni", &[("你", 100), ("尼", 100)]),
            ("hao", &[("好", 100), ("豪", 100)]),
        ]);
        let (model, cleanup) = model("determinism", &"你 好\n尼 豪\n".repeat(10));
        let first = decode(&dictionary, &model, "nihao", 3);
        let second = decode(&dictionary, &model, "nihao", 3);
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        for path in cleanup {
            std::fs::remove_file(path).unwrap();
        }
    }
}
