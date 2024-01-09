use super::*;

pub static NULL_JOURNAL: NullJournal = NullJournal { debug_print: false };

/// The null journal sends all the records into the abyss
#[derive(Debug, Default)]
pub struct NullJournal {
    debug_print: bool,
}

impl ReadableJournal for NullJournal {
    fn read(&self) -> anyhow::Result<Option<JournalEntry<'_>>> {
        Ok(None)
    }

    fn as_restarted(&self) -> anyhow::Result<Box<DynReadableJournal>> {
        Ok(Box::<NullJournal>::default())
    }
}

impl WritableJournal for NullJournal {
    fn write<'a>(&'a self, entry: JournalEntry<'a>) -> anyhow::Result<u64> {
        if self.debug_print {
            tracing::debug!("journal event: {:?}", entry);
        }
        Ok(entry.estimate_size() as u64)
    }
}

impl Journal for NullJournal {
    fn split(self) -> (Box<DynWritableJournal>, Box<DynReadableJournal>) {
        (Box::<NullJournal>::default(), Box::<NullJournal>::default())
    }
}
