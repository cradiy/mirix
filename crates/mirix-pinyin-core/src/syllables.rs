use std::collections::HashMap;

/// Stable only within one [`SyllableTable`] instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SyllableId(u16);

impl SyllableId {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TableError {
    #[error("syllable table is empty")]
    Empty,
    #[error("invalid syllable {0:?}; expected lowercase ASCII letters")]
    Invalid(String),
    #[error("too many syllables")]
    TooLarge,
}

#[derive(Debug, Clone)]
pub struct SyllableTable {
    syllables: Vec<String>,
    exact: HashMap<String, SyllableId>,
    prefixes: HashMap<String, Vec<SyllableId>>,
}

impl SyllableTable {
    pub fn standard() -> Self {
        Self::new(STANDARD_SYLLABLES.split_ascii_whitespace())
            .expect("built-in pinyin syllables must be valid")
    }

    pub fn new<I, S>(syllables: I) -> Result<Self, TableError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut syllables = syllables.into_iter().map(Into::into).collect::<Vec<_>>();
        if syllables.is_empty() {
            return Err(TableError::Empty);
        }
        if let Some(invalid) = syllables.iter().find(|syllable| {
            syllable.is_empty() || !syllable.bytes().all(|byte| byte.is_ascii_lowercase())
        }) {
            return Err(TableError::Invalid(invalid.clone()));
        }
        syllables.sort_unstable();
        syllables.dedup();
        if syllables.len() > usize::from(u16::MAX) + 1 {
            return Err(TableError::TooLarge);
        }

        let mut exact = HashMap::with_capacity(syllables.len());
        let mut prefixes: HashMap<String, Vec<SyllableId>> = HashMap::new();
        for (index, syllable) in syllables.iter().enumerate() {
            let id = SyllableId(index as u16);
            exact.insert(syllable.clone(), id);
            for end in 1..syllable.len() {
                prefixes
                    .entry(syllable[..end].to_owned())
                    .or_default()
                    .push(id);
            }
        }

        Ok(Self {
            syllables,
            exact,
            prefixes,
        })
    }

    pub fn len(&self) -> usize {
        self.syllables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.syllables.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (SyllableId, &str)> {
        self.syllables
            .iter()
            .enumerate()
            .map(|(index, spelling)| (SyllableId(index as u16), spelling.as_str()))
    }

    pub fn id(&self, spelling: &str) -> Option<SyllableId> {
        self.exact.get(spelling).copied()
    }

    pub fn spelling(&self, id: SyllableId) -> Option<&str> {
        self.syllables.get(id.index()).map(String::as_str)
    }

    pub fn completions(&self, prefix: &str) -> &[SyllableId] {
        self.prefixes.get(prefix).map_or(&[], Vec::as_slice)
    }
}

impl Default for SyllableTable {
    fn default() -> Self {
        Self::standard()
    }
}

// Tone-free keyboard spellings used by ordinary full-pinyin input. `v` is
// accepted for ü after n/l; j/q/x/y keep their conventional `u` spelling.
const STANDARD_SYLLABLES: &str = r#"
a ai an ang ao
ba bai ban bang bao bei ben beng bi bian biao bie bin bing bo bu
ca cai can cang cao ce cen ceng cha chai chan chang chao che chen cheng chi chong chou chu chua chuai chuan chuang chui chun chuo ci cong cou cu cuan cui cun cuo
da dai dan dang dao de dei den deng di dia dian diao die ding diu dong dou du duan dui dun duo
e ei en eng er
fa fan fang fei fen feng fo fou fu
ga gai gan gang gao ge gei gen geng gong gou gu gua guai guan guang gui gun guo
ha hai han hang hao he hei hen heng hong hou hu hua huai huan huang hui hun huo
ji jia jian jiang jiao jie jin jing jiong jiu ju juan jue jun
ka kai kan kang kao ke kei ken keng kong kou ku kua kuai kuan kuang kui kun kuo
la lai lan lang lao le lei leng li lia lian liang liao lie lin ling liu lo long lou lu luan lun luo lv lve
ma mai man mang mao me mei men meng mi mian miao mie min ming miu mo mou mu
na nai nan nang nao ne nei nen neng ng ni nian niang niao nie nin ning niu nong nou nu nuan nun nuo nv nve
o ou
pa pai pan pang pao pei pen peng pi pian piao pie pin ping po pou pu
qi qia qian qiang qiao qie qin qing qiong qiu qu quan que qun
ran rang rao re ren reng ri rong rou ru rua ruan rui run ruo
sa sai san sang sao se sen seng sha shai shan shang shao she shei shen sheng shi shou shu shua shuai shuan shuang shui shun shuo si song sou su suan sui sun suo
ta tai tan tang tao te teng ti tian tiao tie ting tong tou tu tuan tui tun tuo
wa wai wan wang wei wen weng wo wu
xi xia xian xiang xiao xie xin xing xiong xiu xu xuan xue xun
ya yan yang yao ye yi yin ying yo yong you yu yuan yue yun
za zai zan zang zao ze zei zen zeng zha zhai zhan zhang zhao zhe zhei zhen zheng zhi zhong zhou zhu zhua zhuai zhuan zhuang zhui zhun zhuo zi zong zou zu zuan zui zun zuo
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_table_has_exact_and_prefix_lookups() {
        let table = SyllableTable::standard();
        assert!(table.len() > 400);
        assert_eq!(table.spelling(table.id("zhe").expect("zhe")), Some("zhe"));
        let completions = table
            .completions("zh")
            .iter()
            .filter_map(|id| table.spelling(*id))
            .collect::<Vec<_>>();
        assert!(completions.contains(&"zhe"));
        assert!(completions.contains(&"zhong"));
    }

    #[test]
    fn custom_table_is_sorted_and_deduplicated() {
        let table = SyllableTable::new(["zhe", "a", "zhe"]).unwrap();
        assert_eq!(table.len(), 2);
        assert_eq!(table.spelling(table.id("a").unwrap()), Some("a"));
    }
}
