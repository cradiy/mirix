use crate::{Lexeme, Lexicon, Reading, ReadingError, SyllableTable, normalize_display_syllable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportIssue {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub lexicon: Lexicon,
    pub accepted: usize,
    pub issues: Vec<ImportIssue>,
}

/// Import normalized text entries in this form:
///
/// `input<TAB>display reading<TAB>text<TAB>frequency`
///
/// `display reading` must contain complete syllables separated by spaces. It
/// determines segmentation; `input` is only a tone-free search key and may be
/// continuous or separated with spaces/apostrophes.
pub fn import_tsv(source: &str, table: &SyllableTable) -> ImportResult {
    import_lines(source, |_, line| import_line(line, table))
}

/// Import the maintained character table:
///
/// `character display-pinyin input-pinyin weight` (whitespace separated).
///
/// The display spelling is authoritative. The third column is retained for
/// inspection, but the keyboard spelling is regenerated from the display so
/// `lü` cannot accidentally become `lu` instead of `lv`.
pub fn import_character_tsv(source: &str, table: &SyllableTable) -> ImportResult {
    import_lines(source, |_, line| import_character_line(line, table))
}

fn import_lines(
    source: &str,
    mut import: impl FnMut(usize, &str) -> Result<Lexeme, String>,
) -> ImportResult {
    let mut entries = Vec::new();
    let mut issues = Vec::new();

    for (index, raw_line) in source.lines().enumerate() {
        let line_number = index + 1;
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        match import(line_number, line) {
            Ok(entry) => entries.push(entry),
            Err(message) => issues.push(ImportIssue {
                line: line_number,
                message,
            }),
        }
    }

    let accepted = entries.len();
    ImportResult {
        lexicon: Lexicon::new(entries),
        accepted,
        issues,
    }
}

fn import_character_line(line: &str, table: &SyllableTable) -> Result<Lexeme, String> {
    let columns = line.split_whitespace().collect::<Vec<_>>();
    if columns.len() != 4 {
        return Err(
            "expected character display-pinyin input-pinyin weight (spaces or tabs)".into(),
        );
    }
    let text = columns[0].trim();
    if text.chars().count() != 1 {
        return Err("character entry must contain exactly one character".into());
    }
    let display = columns[1].trim();
    let input = normalize_display_syllable(display)
        .ok_or_else(|| format!("invalid display syllable {display:?}"))?;
    let reading =
        Reading::new(table, &[input.as_str()], &[display]).map_err(format_reading_error)?;
    let weight = columns[3]
        .parse::<u64>()
        .map_err(|_| "invalid character weight")?;
    Ok(Lexeme::new(reading, text, weight))
}

fn import_line(line: &str, table: &SyllableTable) -> Result<Lexeme, String> {
    let columns = line.split('\t').collect::<Vec<_>>();
    if !(3..=4).contains(&columns.len()) {
        return Err("expected input<TAB>display<TAB>text<TAB>frequency".into());
    }

    let input = normalize_input_key(columns[0])
        .ok_or_else(|| "input reading must contain lowercase ASCII letters".to_owned())?;
    let display_syllables = columns[1]
        .split_whitespace()
        .filter(|syllable| !syllable.is_empty())
        .collect::<Vec<_>>();
    if display_syllables.is_empty() {
        return Err("display reading has no syllables".into());
    }
    let input_syllables = display_syllables
        .iter()
        .map(|syllable| {
            normalize_display_syllable(syllable)
                .ok_or_else(|| format!("invalid display syllable {syllable:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if input_syllables.concat() != input {
        return Err("display reading does not match input reading".into());
    }

    let text = columns[2].trim();
    if text.is_empty() {
        return Err("text is empty".into());
    }
    let frequency = columns
        .get(3)
        .map(|frequency| frequency.trim())
        .filter(|frequency| !frequency.is_empty())
        .unwrap_or("0")
        .parse::<u64>()
        .map_err(|_| "frequency is not an unsigned integer".to_owned())?;

    let input_refs = input_syllables
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let reading =
        Reading::new(table, &input_refs, &display_syllables).map_err(format_reading_error)?;
    Ok(Lexeme::new(reading, text, frequency))
}

fn normalize_input_key(input: &str) -> Option<String> {
    let normalized = input
        .chars()
        .filter(|character| !matches!(character, ' ' | '\''))
        .collect::<String>();
    (!normalized.is_empty()
        && normalized
            .bytes()
            .all(|character| character.is_ascii_lowercase()))
    .then_some(normalized)
}

fn format_reading_error(error: ReadingError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_continuous_keys_using_the_display_segmentation() {
        let table = SyllableTable::standard();
        let result = import_tsv(
            "shurufa\tshū rù fǎ\t输入法\t100\nxi'an\txī ān\t西安\t80\n",
            &table,
        );

        assert!(result.issues.is_empty());
        assert_eq!(result.accepted, 2);
        let reading = [
            table.id("shu").unwrap(),
            table.id("ru").unwrap(),
            table.id("fa").unwrap(),
        ];
        assert_eq!(result.lexicon.exact(&reading)[0].text, "输入法");
        assert_eq!(
            result.lexicon.exact(&reading)[0].reading.display(),
            "shū rù fǎ"
        );
    }

    #[test]
    fn reports_bad_lines_without_discarding_valid_entries() {
        let table = SyllableTable::standard();
        let result = import_tsv(
            "shu\tshū\t书\t20\nshi\tshū\t错音\t10\nmi\tmī\t\t1\n",
            &table,
        );

        assert_eq!(result.accepted, 1);
        assert_eq!(result.lexicon.len(), 1);
        assert_eq!(result.issues.len(), 2);
        assert_eq!(result.issues[0].line, 2);
        assert_eq!(result.issues[1].line, 3);
    }

    #[test]
    fn rejects_unknown_complete_syllables() {
        let table = SyllableTable::standard();
        let result = import_tsv("notreal\tnotrēal\t不存在\t1\n", &table);
        assert_eq!(result.accepted, 0);
        assert_eq!(result.issues.len(), 1);
        assert!(result.issues[0].message.contains("unknown input syllable"));
    }

    #[test]
    fn imports_character_weights_and_normalizes_umlaut_readings() {
        let table = SyllableTable::standard();
        let result = import_character_tsv("爱\tài\tai\t500\n女\tnǚ\tnv\t300\n", &table);
        assert!(result.issues.is_empty());
        assert_eq!(result.accepted, 2);
        assert_eq!(
            result.lexicon.exact(&[table.id("ai").unwrap()])[0].text,
            "爱"
        );
        assert_eq!(
            result.lexicon.exact(&[table.id("nv").unwrap()])[0]
                .reading
                .display(),
            "nǚ"
        );
    }
}
