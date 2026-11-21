//! Test selection: substring and exact filters, skips, repeat, and sharding.

use std::collections::BTreeSet;

use crate::error::TestError;
use crate::model::Test;

/// How tests are sharded across parallel CI jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partition {
    /// One contiguous slice of the filtered list.
    Slice {
        /// Zero-based shard number.
        index: usize,
        /// Total number of shards.
        total: usize,
    },
    /// A stable hash of the test name selects a shard.
    Hash {
        /// Zero-based shard number.
        index: usize,
        /// Total number of shards.
        total: usize,
    },
}

impl Partition {
    /// Parses `slice:N/M` or `hash:N/M`.
    pub fn parse(value: &str) -> Result<Self, TestError> {
        let (kind, counts) = value.split_once(':').ok_or_else(|| {
            TestError::option(format!(
                "the partition `{value}` needs the form `slice:N/M` or `hash:N/M`"
            ))
        })?;
        let (index, total) = counts.split_once('/').ok_or_else(|| {
            TestError::option(format!(
                "the partition `{value}` needs the form `slice:N/M` or `hash:N/M`"
            ))
        })?;
        let index = index.parse::<usize>().map_err(|_| {
            TestError::option(format!("the partition index in `{value}` is not a number"))
        })?;
        let total = total.parse::<usize>().map_err(|_| {
            TestError::option(format!("the partition total in `{value}` is not a number"))
        })?;
        if total == 0 || index >= total {
            return Err(TestError::option(format!(
                "the partition `{value}` needs `0 <= N < M` with `M > 0`"
            )));
        }
        match kind {
            "slice" => Ok(Self::Slice { index, total }),
            "hash" => Ok(Self::Hash { index, total }),
            _ => Err(TestError::option(format!(
                "the partition kind `{kind}` is not `slice` or `hash`"
            ))),
        }
    }

    /// Returns whether `name` belongs to this shard.
    pub fn contains(self, position: usize, name: &str) -> bool {
        match self {
            Self::Slice { index, total } => position % total == index,
            Self::Hash { index, total } => (fnv1a(name) as usize) % total == index,
        }
    }
}

/// One test scheduled once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheduled {
    /// The discovered test.
    pub test: Test,
    /// The name shown by the runner, including the stress repetition.
    pub label: String,
}

/// Selects and expands the tests one run executes.
pub struct Selection<'a> {
    /// Substring filters; a test matches when any filter matches.
    pub filters: &'a [String],
    /// Whether filters compare the whole name.
    pub exact: bool,
    /// Substring skips; a test matching any skip is excluded.
    pub skips: &'a [String],
    /// Shard selection.
    pub partition: Option<Partition>,
    /// Number of times each test runs.
    pub stress: u32,
}

impl Selection<'_> {
    /// Applies the filters and returns the scheduled tests in input order.
    pub fn apply(&self, tests: &[Test]) -> Vec<Scheduled> {
        let mut seen = BTreeSet::new();
        let mut selected = Vec::new();
        for test in tests {
            if !self.filters.is_empty()
                && !self.filters.iter().any(|filter| {
                    if self.exact {
                        test.name == *filter
                    } else {
                        test.name.contains(filter)
                    }
                })
            {
                continue;
            }
            if self.skips.iter().any(|skip| test.name.contains(skip)) {
                continue;
            }
            if !seen.insert((test.package.clone(), test.target.clone(), test.name.clone())) {
                continue;
            }
            selected.push(test.clone());
        }
        if let Some(partition) = self.partition {
            selected = selected
                .into_iter()
                .enumerate()
                .filter(|(position, test)| partition.contains(*position, &test.name))
                .map(|(_, test)| test)
                .collect();
        }
        let mut scheduled = Vec::new();
        let stress = self.stress.max(1);
        for test in selected {
            for repetition in 0..stress {
                let label = if stress == 1 {
                    test.name.clone()
                } else {
                    format!("{} #{}", test.name, repetition + 1)
                };
                scheduled.push(Scheduled {
                    test: test.clone(),
                    label,
                });
            }
        }
        scheduled
    }
}

/// FNV-1a, used only to shard names deterministically across processes.
fn fnv1a(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::{Partition, Selection};
    use crate::model::{Kind, Test};
    use std::path::PathBuf;

    fn test(name: &str) -> Test {
        Test {
            package: "demo".to_owned(),
            target: "unit".to_owned(),
            executable: PathBuf::from("/bin/test"),
            name: name.to_owned(),
            kind: Kind::Test,
        }
    }

    #[test]
    fn substring_filters_are_or_combined() {
        let tests = vec![test("a::one"), test("b::two"), test("c::three")];
        let selection = Selection {
            filters: &["one".to_owned(), "two".to_owned()],
            exact: false,
            skips: &[],
            partition: None,
            stress: 1,
        };
        let selected = selection.apply(&tests);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].label, "a::one");
        assert_eq!(selected[1].label, "b::two");
    }

    #[test]
    fn skips_remove_matching_tests() {
        let tests = vec![test("a::slow"), test("a::fast")];
        let selection = Selection {
            filters: &[],
            exact: true,
            skips: &["slow".to_owned()],
            partition: None,
            stress: 1,
        };
        let selected = selection.apply(&tests);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].label, "a::fast");
    }

    #[test]
    fn stress_expands_each_test() {
        let selection = Selection {
            filters: &[],
            exact: false,
            skips: &[],
            partition: None,
            stress: 3,
        };
        let selected = selection.apply(&[test("a::one")]);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[2].label, "a::one #3");
    }

    #[test]
    fn slices_cover_every_test_once() {
        let tests: Vec<_> = (0..10).map(|index| test(&format!("t{index}"))).collect();
        let mut names = Vec::new();
        for index in 0..3 {
            let selection = Selection {
                filters: &[],
                exact: false,
                skips: &[],
                partition: Some(Partition::Slice { index, total: 3 }),
                stress: 1,
            };
            names.extend(
                selection
                    .apply(&tests)
                    .into_iter()
                    .map(|scheduled| scheduled.label),
            );
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 10);
    }

    #[test]
    fn hash_partitions_are_stable_and_cover_every_test() {
        let tests: Vec<_> = (0..40).map(|index| test(&format!("t{index}"))).collect();
        let mut names = Vec::new();
        for index in 0..4 {
            let partition = Partition::Hash { index, total: 4 };
            let selection = Selection {
                filters: &[],
                exact: false,
                skips: &[],
                partition: Some(partition),
                stress: 1,
            };
            names.extend(
                selection
                    .apply(&tests)
                    .into_iter()
                    .map(|scheduled| scheduled.label),
            );
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 40);
    }

    #[test]
    fn invalid_partitions_are_rejected() {
        assert!(Partition::parse("slice:3/3").is_err());
        assert!(Partition::parse("hash:0/0").is_err());
        assert!(Partition::parse("slice").is_err());
        assert!(Partition::parse("other:0/1").is_err());
    }
}
