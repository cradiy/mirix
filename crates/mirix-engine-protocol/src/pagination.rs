use std::ops::Range;

/// Locate the page containing an absolute candidate index.
pub fn candidate_page(selected: usize, count: usize, ends: &[usize]) -> Range<usize> {
    if count == 0 {
        return 0..0;
    }
    let selected = selected.min(count - 1);
    if ends.is_empty() {
        let start = selected / crate::CANDIDATE_PAGE_SIZE * crate::CANDIDATE_PAGE_SIZE;
        return start..(start + crate::CANDIDATE_PAGE_SIZE).min(count);
    }
    let page = ends.partition_point(|end| *end <= selected);
    let start = if page == 0 { 0 } else { ends[page - 1] };
    start..ends.get(page).copied().unwrap_or(count)
}

pub fn valid_candidate_pages(count: usize, ends: &[usize]) -> bool {
    let mut start = 0;
    for &end in ends {
        if end <= start || end > count || end - start > crate::CANDIDATE_PAGE_SIZE {
            return false;
        }
        start = end;
    }
    start == count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_pages_cover_candidates_without_gaps() {
        assert!(valid_candidate_pages(12, &[3, 8, 12]));
        assert_eq!(candidate_page(2, 12, &[3, 8, 12]), 0..3);
        assert_eq!(candidate_page(3, 12, &[3, 8, 12]), 3..8);
        assert_eq!(candidate_page(99, 12, &[3, 8, 12]), 8..12);
        for ends in [
            vec![0, 12],
            vec![3, 3, 12],
            vec![13],
            vec![10, 12],
            vec![3, 8],
        ] {
            assert!(!valid_candidate_pages(12, &ends));
        }
    }
}
