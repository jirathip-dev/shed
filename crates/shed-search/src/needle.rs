//! Fixed text in bytes, by Horspool's skip table, in exact or any ASCII case.

pub(crate) struct Needle {
    bytes: Vec<u8>,
    skip: [usize; 256],
    fold: bool,
}

impl Needle {
    pub(crate) fn new(text: &str, fold: bool) -> Self {
        let bytes = if fold {
            text.to_ascii_lowercase().into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        let mut skip = [bytes.len(); 256];
        let last = bytes.len().saturating_sub(1);
        for (i, &b) in bytes.iter().enumerate().take(last) {
            let shift = last - i;
            for b in [b, if fold { b.to_ascii_uppercase() } else { b }] {
                if let Some(slot) = skip.get_mut(usize::from(b)) {
                    *slot = shift;
                }
            }
        }
        Self { bytes, skip, fold }
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn find(&self, hay: &[u8], from: usize) -> Option<usize> {
        let m = self.bytes.len();
        let (&want, head) = self.bytes.split_last()?;
        let mut end = from + m - 1;
        while let Some(&c) = hay.get(end) {
            if self.fold_byte(c) == want {
                let start = end + 1 - m;
                let window = hay.get(start..end)?;
                if window
                    .iter()
                    .zip(head)
                    .all(|(&a, &b)| self.fold_byte(a) == b)
                {
                    return Some(start);
                }
            }
            end += self.skip.get(usize::from(c)).copied().unwrap_or(m);
        }
        None
    }

    fn fold_byte(&self, b: u8) -> u8 {
        if self.fold { b.to_ascii_lowercase() } else { b }
    }
}

#[cfg(test)]
mod tests {
    use super::Needle;

    #[test]
    fn finds_every_occurrence_in_either_case() {
        let hay = b"Wait, wait: WAIT and wai";
        let all = |n: &Needle| {
            let mut at = Vec::new();
            let mut from = 0;
            while let Some(i) = n.find(hay, from) {
                at.push(i);
                from = i + 1;
            }
            at
        };
        assert_eq!(all(&Needle::new("wait", false)), [6]);
        assert_eq!(all(&Needle::new("wait", true)), [0, 6, 12]);
        assert_eq!(all(&Needle::new("Wai", true)), [0, 6, 12, 21]);
        assert_eq!(all(&Needle::new("x", false)), Vec::<usize>::new());
        assert_eq!(Needle::new("and", false).find(b"an", 0), None);
    }
}
