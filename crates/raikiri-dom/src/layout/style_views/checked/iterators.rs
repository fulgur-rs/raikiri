use super::compare::{CheckedValue, Diagnostic};
use crate::LayoutStyle;
use raikiri_layout_style::{Areas, GridRepetition, LineNames, TemplateTracks, Tracks};
use raikiri_style::SmolStr;
use taffy::{GenericGridTemplateComponent as Component, GenericRepetition, GridContainerStyle};

type LegacyTemplate<'a> = <LayoutStyle as GridContainerStyle>::TemplateTrackList<'a>;
type LegacyTracks<'a> = <LayoutStyle as GridContainerStyle>::AutoTrackList<'a>;
type LegacyNames<'a> = <LayoutStyle as GridContainerStyle>::TemplateLineNames<'a>;
type LegacyAreas<'a> = <LayoutStyle as GridContainerStyle>::GridTemplateAreas<'a>;
type LegacyRepeat<'a> = <LayoutStyle as GridContainerStyle>::Repetition<'a>;
pub(crate) type CheckedTracks<'a> = CheckedIter<'a, LegacyTracks<'a>, Tracks<'a>>;
pub(crate) type CheckedAreas<'a> = CheckedIter<'a, LegacyAreas<'a>, Areas<'a>>;
type CheckedNameSet<'a> =
    CheckedIter<'a, std::slice::Iter<'a, SmolStr>, std::slice::Iter<'a, SmolStr>>;

#[derive(Clone)]
pub(crate) struct CheckedIter<'a, L: Iterator, R: Iterator<Item = L::Item>> {
    old: L,
    view: Option<R>,
    diag: Diagnostic<'a>,
    index: usize,
}
impl<'a, L: Iterator, R: Iterator<Item = L::Item>> CheckedIter<'a, L, R> {
    pub(super) fn new(old: L, view: Option<R>, diag: Diagnostic<'a>) -> Self {
        Self {
            old,
            view,
            diag,
            index: 0,
        }
    }
}
impl<L, R> Iterator for CheckedIter<'_, L, R>
where
    L: ExactSizeIterator,
    R: ExactSizeIterator<Item = L::Item>,
    L::Item: CheckedValue,
{
    type Item = L::Item;
    fn next(&mut self) -> Option<Self::Item> {
        let old = self.old.next();
        if let Some(view) = &mut self.view {
            let diag = self.diag.indexed(self.index);
            diag.element();
            old.check(&view.next(), diag);
        }
        self.index += 1;
        old
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if let Some(view) = &self.view {
            self.diag.element();
            self.diag.equal(&self.old.len(), &view.len());
        }
        self.old.size_hint()
    }
}
impl<L, R> ExactSizeIterator for CheckedIter<'_, L, R>
where
    L: ExactSizeIterator,
    R: ExactSizeIterator<Item = L::Item>,
    L::Item: CheckedValue,
{
}

#[derive(Clone)]
pub(crate) struct CheckedNames<'a> {
    old: LegacyNames<'a>,
    view: Option<LineNames<'a>>,
    diag: Diagnostic<'a>,
    index: usize,
}
impl<'a> CheckedNames<'a> {
    pub(super) fn new(
        old: LegacyNames<'a>,
        view: Option<LineNames<'a>>,
        diag: Diagnostic<'a>,
    ) -> Self {
        Self {
            old,
            view,
            diag,
            index: 0,
        }
    }
}
impl<'a> Iterator for CheckedNames<'a> {
    type Item = CheckedNameSet<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let old = self.old.next();
        let view = self.view.as_mut().and_then(Iterator::next);
        let diag = self.diag.indexed(self.index);
        self.index += 1;
        if self.view.is_some() {
            diag.element();
            diag.equal(&old.is_some(), &view.is_some());
        }
        old.map(|old| CheckedIter::new(old, view, diag))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if let Some(view) = &self.view {
            self.diag.element();
            self.diag.equal(&self.old.len(), &view.len());
        }
        self.old.size_hint()
    }
}
impl ExactSizeIterator for CheckedNames<'_> {}
impl<'a> taffy::TemplateLineNames<'a, SmolStr> for CheckedNames<'a> {
    type LineNameSet<'b>
        = CheckedNameSet<'b>
    where
        Self: 'b;
}

#[derive(Clone, Copy)]
pub(crate) struct CheckedRepeat<'a> {
    old: LegacyRepeat<'a>,
    view: Option<GridRepetition<'a>>,
    diag: Diagnostic<'a>,
}
impl GenericRepetition for CheckedRepeat<'_> {
    type CustomIdent = SmolStr;
    type RepetitionTrackList<'a>
        = CheckedTracks<'a>
    where
        Self: 'a;
    type TemplateLineNames<'a>
        = CheckedNames<'a>
    where
        Self: 'a;
    fn count(&self) -> taffy::RepetitionCount {
        let old = self.old.count();
        if let Some(view) = &self.view {
            self.diag.getter("GenericRepetition::count");
            self.diag.element();
            old.check(&view.count(), self.diag.component("repeat.count"));
        }
        old
    }
    fn tracks(&self) -> CheckedTracks<'_> {
        if self.view.is_some() {
            self.diag.getter("GenericRepetition::tracks");
        }
        CheckedIter::new(
            self.old.tracks(),
            self.view.as_ref().map(|view| view.tracks()),
            self.diag.component("repeat.tracks"),
        )
    }
    fn lines_names(&self) -> CheckedNames<'_> {
        if self.view.is_some() {
            self.diag.getter("GenericRepetition::lines_names");
        }
        CheckedNames::new(
            self.old.lines_names(),
            self.view.as_ref().map(|view| view.lines_names()),
            self.diag.component("repeat.names"),
        )
    }
}

#[derive(Clone)]
pub(crate) struct CheckedTemplate<'a> {
    old: LegacyTemplate<'a>,
    view: Option<TemplateTracks<'a>>,
    diag: Diagnostic<'a>,
    index: usize,
}
impl<'a> CheckedTemplate<'a> {
    pub(super) fn new(
        old: LegacyTemplate<'a>,
        view: Option<TemplateTracks<'a>>,
        diag: Diagnostic<'a>,
    ) -> Self {
        Self {
            old,
            view,
            diag,
            index: 0,
        }
    }
}
fn component_kind<R: GenericRepetition<CustomIdent = SmolStr>>(
    value: &Option<Component<SmolStr, R>>,
) -> &'static str {
    match value {
        None => "end",
        Some(Component::Single(_)) => "single",
        Some(Component::Repeat(_)) => "repeat",
    }
}
impl<'a> Iterator for CheckedTemplate<'a> {
    type Item = Component<SmolStr, CheckedRepeat<'a>>;
    fn next(&mut self) -> Option<Self::Item> {
        let old = self.old.next();
        let diag = self.diag.indexed(self.index);
        self.index += 1;
        if let Some(view) = &mut self.view {
            let view = view.next();
            diag.element();
            match (old, view) {
                (None, None) => None,
                (Some(Component::Single(old)), Some(Component::Single(view))) => {
                    old.check(&view, diag);
                    Some(Component::Single(old))
                }
                (Some(Component::Repeat(old)), Some(Component::Repeat(view))) => {
                    Some(Component::Repeat(CheckedRepeat {
                        old,
                        view: Some(view),
                        diag,
                    }))
                }
                (old, view) => diag.mismatch(&component_kind(&old), &component_kind(&view)),
            }
        } else {
            old.map(|old| match old {
                Component::Single(track) => Component::Single(track),
                Component::Repeat(old) => Component::Repeat(CheckedRepeat {
                    old,
                    view: None,
                    diag,
                }),
            })
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if let Some(view) = &self.view {
            self.diag.element();
            self.diag.equal(&self.old.len(), &view.len());
        }
        self.old.size_hint()
    }
}
impl ExactSizeIterator for CheckedTemplate<'_> {}
