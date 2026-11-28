//! Safe `!Send`/`!Sync` wrapper around `GhosttySearch`.

use std::marker::PhantomData;
use std::ptr;

use ghostty_vt_sys as sys;

use super::terminal::{GridPointTag, Selection, SelectionSpan, Terminal};
use super::{check, Result, VtError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchStatus {
    Running,
    FeedRequired,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchScroll {
    IfNeeded,
    None,
}

pub(crate) struct Search {
    raw: sys::GhosttySearch,
    owner_id: u64,
    synced_gen: Option<u64>,
    _not_send_sync: PhantomData<*mut ()>,
}

impl Search {
    pub(crate) fn new(term: &Terminal) -> Result<Self> {
        let mut raw: sys::GhosttySearch = ptr::null_mut();
        check(unsafe { sys::ghostty_search_new(ptr::null(), &mut raw, term.raw()) })?;
        Ok(Self {
            raw,
            owner_id: term.id(),
            synced_gen: Some(term.mutation_gen()),
            _not_send_sync: PhantomData,
        })
    }

    fn ensure_owner(&self, term: &Terminal) -> Result<()> {
        if self.owner_id != term.id() {
            return Err(VtError::InvalidValue);
        }
        Ok(())
    }

    fn ensure_synced(&self, term: &Terminal) -> Result<()> {
        self.ensure_owner(term)?;
        if self.synced_gen != Some(term.mutation_gen()) {
            return Err(VtError::InvalidValue);
        }
        Ok(())
    }

    pub(crate) fn set_needle(&mut self, term: &Terminal, needle: Option<&str>) -> Result<()> {
        self.ensure_owner(term)?;
        match needle {
            Some(text) if !text.is_empty() => {
                let s = sys::GhosttyString {
                    ptr: text.as_ptr(),
                    len: text.len(),
                };
                check(unsafe {
                    sys::ghostty_search_set(
                        self.raw,
                        sys::GHOSTTY_SEARCH_OPT_NEEDLE,
                        (&raw const s).cast(),
                    )
                })?;
                if self.status() != SearchStatus::Complete {
                    self.synced_gen = None;
                }
                Ok(())
            }
            _ => {
                check(unsafe {
                    sys::ghostty_search_set(self.raw, sys::GHOSTTY_SEARCH_OPT_NEEDLE, ptr::null())
                })?;
                self.synced_gen = Some(term.mutation_gen());
                Ok(())
            }
        }
    }

    pub(crate) fn set_select_scroll(&mut self, scroll: SearchScroll) -> Result<()> {
        let raw = match scroll {
            SearchScroll::IfNeeded => sys::GHOSTTY_SEARCH_SCROLL_IF_NEEDED,
            SearchScroll::None => sys::GHOSTTY_SEARCH_SCROLL_NONE,
        };
        check(unsafe {
            sys::ghostty_search_set(
                self.raw,
                sys::GHOSTTY_SEARCH_OPT_SELECT_SCROLL,
                (&raw const raw).cast(),
            )
        })
    }

    pub(crate) fn run(&mut self, term: &Terminal) -> Result<()> {
        self.ensure_owner(term)?;
        self.synced_gen = None;
        check(unsafe { sys::ghostty_search_run(self.raw) })?;
        self.synced_gen = Some(term.mutation_gen());
        Ok(())
    }

    pub(crate) fn feed(&mut self, term: &Terminal) -> Result<()> {
        self.ensure_owner(term)?;
        self.synced_gen = None;
        check(unsafe { sys::ghostty_search_feed(self.raw) })?;
        self.synced_gen = Some(term.mutation_gen());
        Ok(())
    }

    pub(crate) fn tick(&mut self) -> Result<SearchStatus> {
        let mut status: sys::GhosttySearchStatus = sys::GHOSTTY_SEARCH_STATUS_COMPLETE;
        check(unsafe { sys::ghostty_search_tick(self.raw, &mut status) })?;
        Ok(status_from_raw(status))
    }

    pub(crate) fn status(&self) -> SearchStatus {
        let mut status: sys::GhosttySearchStatus = sys::GHOSTTY_SEARCH_STATUS_COMPLETE;
        unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_STATUS,
                (&raw mut status).cast(),
            );
        }
        status_from_raw(status)
    }

    pub(crate) fn select_next(&mut self, term: &mut Terminal) -> Result<bool> {
        self.select(term, sys::ghostty_search_select_next)
    }

    pub(crate) fn select_prev(&mut self, term: &mut Terminal) -> Result<bool> {
        self.select(term, sys::ghostty_search_select_prev)
    }

    fn select(
        &mut self,
        term: &mut Terminal,
        op: unsafe extern "C" fn(sys::GhosttySearch) -> sys::GhosttyResult,
    ) -> Result<bool> {
        self.ensure_owner(term)?;
        self.synced_gen = None;
        // Native select_* reconciles screens with `active_dirty = false` and may
        // scroll after its internal feed, which can leave cached viewport matches
        // stale after in-place active edits or viewport scrolling. Feed before
        // and after navigation so both active-area and viewport results stay current.
        check(unsafe { sys::ghostty_search_feed(self.raw) })?;
        let rc = unsafe { op(self.raw) };
        term.bump_mutation_gen();
        let selected = match rc {
            sys::GHOSTTY_SUCCESS => true,
            sys::GHOSTTY_NO_VALUE => false,
            other => return Err(check(other).unwrap_err()),
        };
        check(unsafe { sys::ghostty_search_feed(self.raw) })?;
        self.synced_gen = Some(term.mutation_gen());
        Ok(selected)
    }

    pub(crate) fn total_matches(&self) -> usize {
        let mut total: usize = 0;
        unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_TOTAL_MATCHES,
                (&raw mut total).cast(),
            );
        }
        total
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        let mut idx: usize = 0;
        let rc = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_SELECTED_INDEX,
                (&raw mut idx).cast(),
            )
        };
        (rc == sys::GHOSTTY_SUCCESS).then_some(idx)
    }

    pub(crate) fn selected_match<'a>(&self, term: &'a Terminal) -> Result<Option<Selection<'a>>> {
        self.ensure_synced(term)?;
        let mut raw = sys::GhosttySelection::default();
        let rc = unsafe {
            sys::ghostty_search_get(
                self.raw,
                sys::GHOSTTY_SEARCH_DATA_SELECTED_MATCH,
                (&raw mut raw).cast(),
            )
        };
        match rc {
            sys::GHOSTTY_SUCCESS => Ok(Some(unsafe { term.wrap_selection(raw) })),
            sys::GHOSTTY_NO_VALUE => Ok(None),
            other => Err(check(other).unwrap_err()),
        }
    }

    pub(crate) fn selected_match_span(
        &self,
        term: &Terminal,
        tag: GridPointTag,
    ) -> Result<Option<SelectionSpan>> {
        let Some(sel) = self.selected_match(term)? else {
            return Ok(None);
        };
        term.selection_span(&sel, tag)
    }

    pub(crate) fn matches<'a>(&self, term: &'a Terminal) -> Result<Vec<Selection<'a>>> {
        self.read_selection_buffer(term, sys::GHOSTTY_SEARCH_DATA_MATCHES)
    }

    pub(crate) fn viewport_matches<'a>(&self, term: &'a Terminal) -> Result<Vec<Selection<'a>>> {
        self.read_selection_buffer(term, sys::GHOSTTY_SEARCH_DATA_VIEWPORT_MATCHES)
    }

    pub(crate) fn matches_spans(
        &self,
        term: &Terminal,
        tag: GridPointTag,
    ) -> Result<Vec<SelectionSpan>> {
        let sels = self.matches(term)?;
        let mut out = Vec::with_capacity(sels.len());
        for sel in sels {
            if let Some(span) = term.selection_span(&sel, tag)? {
                out.push(span);
            }
        }
        Ok(out)
    }

    pub(crate) fn viewport_matches_spans(
        &self,
        term: &Terminal,
        tag: GridPointTag,
    ) -> Result<Vec<SelectionSpan>> {
        let sels = self.viewport_matches(term)?;
        let mut out = Vec::with_capacity(sels.len());
        for sel in sels {
            if let Some(span) = term.selection_span(&sel, tag)? {
                out.push(span);
            }
        }
        Ok(out)
    }

    fn read_selection_buffer<'a>(
        &self,
        term: &'a Terminal,
        key: sys::GhosttySearchData,
    ) -> Result<Vec<Selection<'a>>> {
        self.ensure_synced(term)?;
        let mut query = sys::GhosttySelectionBuffer {
            ptr: ptr::null_mut(),
            cap: 0,
            len: 0,
        };
        let rc = unsafe { sys::ghostty_search_get(self.raw, key, (&raw mut query).cast()) };
        match rc {
            sys::GHOSTTY_SUCCESS if query.len == 0 => Ok(Vec::new()),
            sys::GHOSTTY_OUT_OF_SPACE => {
                let mut buf = vec![sys::GhosttySelection::default(); query.len];
                let mut filled = sys::GhosttySelectionBuffer {
                    ptr: buf.as_mut_ptr(),
                    cap: buf.len(),
                    len: 0,
                };
                check(unsafe { sys::ghostty_search_get(self.raw, key, (&raw mut filled).cast()) })?;
                buf.truncate(filled.len);
                Ok(buf
                    .into_iter()
                    .map(|raw| unsafe { term.wrap_selection(raw) })
                    .collect())
            }
            other => Err(check(other).unwrap_err()),
        }
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        unsafe { sys::ghostty_search_free(self.raw) };
    }
}

fn status_from_raw(raw: sys::GhosttySearchStatus) -> SearchStatus {
    match raw {
        sys::GHOSTTY_SEARCH_STATUS_RUNNING => SearchStatus::Running,
        sys::GHOSTTY_SEARCH_STATUS_FEED_REQUIRED => SearchStatus::FeedRequired,
        _ => SearchStatus::Complete,
    }
}
