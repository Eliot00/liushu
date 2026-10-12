//! 合法拼音音节表与拼音切分格。
//!
//! 音节表取自本项目已依赖的 `pinyin` crate（MIT / Apache-2.0）构建时生成的读音表，去重排序后
//! 共 420 条——它比手写的音韵表可靠：`scripts/legal_pinyins.py` 那种「声母 × 韵母」笛卡尔积既缺
//! `xian` / `liang` / `jiong` 这类真音节，又会造出 `zuang` 这种不存在的音节。
//!
//! 表按字节序排好，查询走二分；音节都是纯 ASCII 小写字母，因此按字节切分一定落在字符边界上。
//! 嗯 / 呣 这类叹词音节（`n` / `ng` / `m` / `hm` / `hng`）保留：它们是真能敲出来的输入。

/// 最长音节的字节数，枚举候选跨度时的上界。
pub const MAX_SYLLABLE_LEN: usize = 6;

/// 合法音节，420 条，按字节序升序。
pub const SYLLABLES: [&str; 420] = [
    "a", "ai", "an", "ang", "ao", "ba", "bai", "ban", "bang", "bao", "bei", "ben", "beng", "bi",
    "bian", "biang", "biao", "bie", "bin", "bing", "bo", "bu", "ca", "cai", "can", "cang", "cao",
    "ce", "cei", "cen", "ceng", "cha", "chai", "chan", "chang", "chao", "che", "chen", "cheng",
    "chi", "chong", "chou", "chu", "chua", "chuai", "chuan", "chuang", "chui", "chun", "chuo",
    "ci", "cong", "cou", "cu", "cuan", "cui", "cun", "cuo", "da", "dai", "dan", "dang", "dao",
    "de", "dei", "den", "deng", "di", "dia", "dian", "diao", "die", "din", "ding", "diu", "dong",
    "dou", "du", "duan", "dui", "dun", "duo", "e", "ei", "en", "eng", "er", "fa", "fan", "fang",
    "fei", "fen", "feng", "fiao", "fo", "fou", "fu", "ga", "gai", "gan", "gang", "gao", "ge",
    "gei", "gen", "geng", "gong", "gou", "gu", "gua", "guai", "guan", "guang", "gui", "gun", "guo",
    "ha", "hai", "han", "hang", "hao", "he", "hei", "hen", "heng", "hm", "hng", "hong", "hou",
    "hu", "hua", "huai", "huan", "huang", "hui", "hun", "huo", "ji", "jia", "jian", "jiang",
    "jiao", "jie", "jin", "jing", "jiong", "jiu", "ju", "juan", "jue", "jun", "ka", "kai", "kan",
    "kang", "kao", "ke", "kei", "ken", "keng", "kong", "kou", "ku", "kua", "kuai", "kuan", "kuang",
    "kui", "kun", "kuo", "la", "lai", "lan", "lang", "lao", "le", "lei", "len", "leng", "li",
    "lia", "lian", "liang", "liao", "lie", "lin", "ling", "liu", "lo", "long", "lou", "lu", "luan",
    "lun", "luo", "m", "ma", "mai", "man", "mang", "mao", "me", "mei", "men", "meng", "mi", "mian",
    "miao", "mie", "min", "ming", "miu", "mo", "mou", "mu", "n", "na", "nai", "nan", "nang", "nao",
    "ne", "nei", "nen", "neng", "ng", "ni", "nia", "nian", "niang", "niao", "nie", "nin", "ning",
    "niu", "nong", "nou", "nu", "nuan", "nun", "nuo", "o", "ou", "pa", "pai", "pan", "pang", "pao",
    "pei", "pen", "peng", "pi", "pian", "piao", "pie", "pin", "ping", "po", "pou", "pu", "qi",
    "qia", "qian", "qiang", "qiao", "qie", "qin", "qing", "qiong", "qiu", "qu", "quan", "que",
    "qun", "ran", "rang", "rao", "re", "ren", "reng", "ri", "rong", "rou", "ru", "rua", "ruan",
    "rui", "run", "ruo", "sa", "sai", "san", "sang", "sao", "se", "sen", "seng", "sha", "shai",
    "shan", "shang", "shao", "she", "shei", "shen", "sheng", "shi", "shou", "shu", "shua", "shuai",
    "shuan", "shuang", "shui", "shun", "shuo", "si", "song", "sou", "su", "suan", "sui", "sun",
    "suo", "ta", "tai", "tan", "tang", "tao", "te", "tei", "teng", "ti", "tian", "tiao", "tie",
    "ting", "tong", "tou", "tu", "tuan", "tui", "tun", "tuo", "wa", "wai", "wan", "wang", "wei",
    "wen", "weng", "wo", "wong", "wu", "xi", "xia", "xian", "xiang", "xiao", "xie", "xin", "xing",
    "xiong", "xiu", "xu", "xuan", "xue", "xun", "ya", "yan", "yang", "yao", "ye", "yi", "yin",
    "ying", "yo", "yong", "you", "yu", "yuan", "yue", "yun", "za", "zai", "zan", "zang", "zao",
    "ze", "zei", "zen", "zeng", "zha", "zhai", "zhan", "zhang", "zhao", "zhe", "zhei", "zhen",
    "zheng", "zhi", "zhong", "zhou", "zhu", "zhua", "zhuai", "zhuan", "zhuang", "zhui", "zhun",
    "zhuo", "zi", "zong", "zou", "zu", "zuan", "zui", "zun", "zuo",
];

/// 是不是一个合法音节。
pub fn is_syllable(candidate: &str) -> bool {
    SYLLABLES.binary_search(&candidate).is_ok()
}

/// `code[from..]` 起第一个音节的结束位置，按升序。
///
/// 音节都是纯 ASCII 小写字母，所以按字节推进一定落在字符边界上；遇到非小写字母就停，
/// 别把数字或标点当成音节的一部分。
pub fn spans(code: &str, from: usize) -> Vec<usize> {
    let bytes = code.as_bytes();
    let mut ends = Vec::new();
    let mut end = from;
    while end < bytes.len() && end - from < MAX_SYLLABLE_LEN {
        if !bytes[end].is_ascii_lowercase() {
            break;
        }
        end += 1;
        if is_syllable(&code[from..end]) {
            ends.push(end);
        }
    }
    ends
}

/// 从 `from` 起切 1..=`max_syllables` 个音节能到的所有位置（不含 `from`），按升序。
///
/// 按层展开：某一层已经到过的位置，其后续展开在剩余步数更多的层里已经做过，所以只展开一次。
pub fn reachable(code: &str, from: usize, max_syllables: usize) -> Vec<usize> {
    let mut reached: Vec<usize> = Vec::new();
    let mut frontier = vec![from];
    for _ in 0..max_syllables {
        let mut next = Vec::new();
        for &start in &frontier {
            for end in spans(code, start) {
                if !reached.contains(&end) {
                    reached.push(end);
                    next.push(end);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    reached.sort_unstable();
    reached
}

/// 从 0 起能切到的所有位置（含 0），按升序。
pub fn positions(code: &str) -> Vec<usize> {
    let mut reached = vec![0usize];
    let mut frontier = vec![0usize];
    while let Some(start) = frontier.pop() {
        for end in spans(code, start) {
            if !reached.contains(&end) {
                reached.push(end);
                frontier.push(end);
            }
        }
    }
    reached.sort_unstable();
    reached
}

/// 整串能不能切成合法音节序列。
pub fn is_sequence(code: &str) -> bool {
    positions(code).contains(&code.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_ascii_lowercase() {
        // 二分查找要求有序；顺带保证切分时按字节推进是安全的。
        assert!(SYLLABLES.windows(2).all(|w| w[0] < w[1]));
        assert!(
            SYLLABLES
                .iter()
                .all(|syllable| syllable.bytes().all(|byte| byte.is_ascii_lowercase()))
        );
        assert_eq!(SYLLABLES.len(), 420);
    }

    #[test]
    fn recognises_syllables_and_rejects_non_syllables() {
        assert!(is_syllable("ni"));
        assert!(is_syllable("xian"));
        assert!(is_syllable("shuang"));
        assert!(is_syllable("ng"));
        assert!(!is_syllable("nihao"));
        assert!(!is_syllable("q"));
        assert!(!is_syllable(""));
        assert!(!is_syllable("NI"));
    }

    #[test]
    fn spans_offer_every_syllable_that_fits() {
        // `xian` 里 `xi` / `xia` / `xian` 都是音节——词图要每条路都给。
        assert_eq!(spans("xian", 0), vec![2, 3, 4]);
        // `n`（嗯）也是一个音节，所以 `nihao` 有两种开头。
        assert_eq!(spans("nihao", 0), vec![1, 2]);
        assert_eq!(spans("nihao", 2), vec![4, 5]);
        // 数字不进音节：`ni3` 只切得出 `n` / `ni`，从数字起一个音节都没有。
        assert_eq!(spans("ni3", 0), vec![1, 2]);
        assert!(spans("ni3", 2).is_empty());
    }

    #[test]
    fn reachable_counts_syllables_not_bytes() {
        assert_eq!(reachable("xian", 0, 1), vec![2, 3, 4]);
        assert_eq!(reachable("xian", 0, 2), vec![2, 3, 4]);
        assert_eq!(reachable("nihaoma", 0, 2), vec![1, 2, 4, 5]);
        // 第三步才能走完 `ma`，`m`（呣）+ `a` 也在这一步到 7。
        assert_eq!(reachable("nihaoma", 0, 3), vec![1, 2, 4, 5, 6, 7]);
    }

    #[test]
    fn a_typo_makes_the_whole_string_unsegmentable() {
        assert!(is_sequence("nihaoshijie"));
        assert!(is_sequence(""));
        assert!(!is_sequence("nihaoq"));
        assert!(!is_sequence("n1hao"));
        assert_eq!(positions("nihao"), vec![0, 1, 2, 4, 5]);
    }
}
