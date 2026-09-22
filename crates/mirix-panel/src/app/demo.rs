use mirix_core::{CandidateId, PanelCandidate, PanelSnapshot};

pub(super) fn snapshot() -> PanelSnapshot {
    PanelSnapshot {
        punctuation_mode: Some(mirix_core::PunctuationMode::Full),
        context_id: None,
        preedit: "zhong wen shu ru".into(),
        preedit_cursor: 16,
        preedit_selection: None,
        panel_composition: mirix_core::PanelPreedit::default(),
        candidates: [
            ("1", "中文输入法", Some("zhongwenshurufa")),
            ("2", "中文", Some("zhongwen")),
            ("3", "输入法", Some("shurufa")),
            ("4", "输入", Some("shuru")),
            ("5", "文字", Some("wenzi")),
            ("6", "候选框", Some("houxuankuang")),
            ("7", "打字", Some("dazi")),
            ("8", "拼音", Some("pinyin")),
            ("9", "键盘", Some("jianpan")),
            ("1", "下一页", Some("xiayiye")),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (label, text, comment))| PanelCandidate {
            keep_open: false,
            id: CandidateId(index as u64 + 1),
            label: label.into(),
            text: text.into(),
            comment: comment.map(str::to_owned),
            reading: None,
        })
        .collect(),

        selected: 0,
        visible: true,
    }
}
