use gpui::{App, Bounds, IntoElement, ParentElement, Pixels, Styled as _, Window, canvas};

use crate::TextSelectionScopeId;

/// Extends a GPUI parent element with post-layout prepaint observation.
pub trait ElementExt: ParentElement + Sized {
    /// Marks this element subtree as belonging to a text-selection scope.
    fn text_selection_scope(self, scope: TextSelectionScopeId) -> impl IntoElement
    where
        Self: IntoElement,
    {
        crate::text_selection::text_selection_scope(scope, self)
    }

    /// Invokes `callback` during prepaint with this element's resolved bounds.
    fn on_prepaint<F>(self, callback: F) -> Self
    where
        F: FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static,
    {
        self.child(
            canvas(
                move |bounds, window, cx| callback(bounds, window, cx),
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
    }
}

impl<T: ParentElement> ElementExt for T {}

/// Requests the layout of `text` in a layout node of its own, for an element
/// that reads the text's [`gpui::TextLayout`].
///
/// A block element takes text among its children into its own inline
/// paragraph and lays it out there, and the text's own layout is never
/// measured: hit testing, selection and highlights read from it then panic.
/// The node returned is not text, so a block keeps it whole, as a block
/// child, and the text in it is measured as it is anywhere else.
pub fn request_text_layout(
    text: &mut gpui::StyledText,
    id: Option<&gpui::GlobalElementId>,
    inspector_id: Option<&gpui::InspectorElementId>,
    window: &mut Window,
    cx: &mut App,
) -> gpui::LayoutId {
    let (text_layout_id, ()) = gpui::Element::request_layout(text, id, inspector_id, window, cx);
    window.request_layout(gpui::Style::default(), [text_layout_id], cx)
}
