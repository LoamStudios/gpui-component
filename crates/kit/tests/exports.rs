//! Every path `gpui-kit` promises resolves, under every feature set.
//!
//! These are compile-time checks: each `use` or type alias fails to build if
//! a re-export goes missing or moves, which is the regression that matters
//! for an umbrella crate.

#![allow(unused_imports, dead_code)]

/// `use gpui_kit::*;` alone is GPUI, so a file needs nothing else for it.
mod glob_is_gpui {
    use gpui_kit::*;
    // GPUI CE's action macros expand to unanchored paths; an explicit alias
    // keeps them off the ambiguous extern-crate name.
    use gpui_kit as gpui;

    type Element = Div;
    type Window_ = Window;
    type App_ = App;
    type Geometry = Size<Pixels>;

    actions!(exports, [Quit]);

    #[derive(Action, Clone, PartialEq)]
    struct DerivedAction;

    #[derive(Render)]
    struct DerivedRender;

    #[derive(IntoElement)]
    struct DerivedElement;

    impl RenderOnce for DerivedElement {
        fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
            div()
        }
    }

    fn build() -> impl IntoElement {
        div().child("hello").bg(red())
    }

    fn startup(cx: &mut App) {
        gpui_kit::init(cx);
        let _ = gpui_kit::application;
    }
}

/// Importing only the macro must not require a transitive crate named `gpui`.
mod selective_actions_import {
    use gpui_kit::actions;

    actions!(exports, [SelectivelyImportedAction]);

    fn assert_action<T: gpui_kit::Action>() {}

    fn action_is_from_the_facade() {
        assert_action::<SelectivelyImportedAction>();
    }
}

/// `gpui_kit::gpui` is hidden but kept, for code that keeps `gpui::…` paths.
mod gpui_by_name {
    use gpui_kit::*;

    type Through = gpui::Window;
    type Spelled = gpui_kit::gpui::Window;
}

/// The layers that are always present.
mod always {
    type Base = gpui_kit::base::Button;
    const _APPLICATION: fn() -> gpui_kit::Application = gpui_kit::application;
    const _PLATFORM_APPLICATION: fn() -> gpui_kit::Application = gpui_kit::platform::application;
    const _INIT: fn(&mut gpui_kit::App) = gpui_kit::init;
}

#[cfg(feature = "component")]
mod component {
    use gpui_kit::component::button::*;
    use gpui_kit::component::plot::{IntoPlot, Plot};
    use gpui_kit::component::{ActiveTheme, Root, Size};
    use gpui_kit::*;

    type Sizing = Size;

    // Deriving with `IntoPlot` is exercised end to end by the story crate's
    // charts; this crate's own integration tests resolve macros to `crate`,
    // where the derive cannot expand, so the export is checked by the import.
    #[allow(unused_imports)]
    use gpui_kit::component::plot::IntoPlot as IntoPlotDeriveIsExported;

    fn theme(cx: &App) -> Hsla {
        cx.theme().background
    }

    fn button() -> Button {
        Button::new("ok").primary()
    }

    const _INIT: fn(&mut App) = gpui_kit::component::init;
}

#[cfg(feature = "assets")]
mod assets {
    type Assets = gpui_kit::assets::Assets;
}

/// The opt-in facade feature enables GPUI's feature-gated profiler API.
#[cfg(feature = "profiler")]
use gpui_kit::profiler::WindowProfiler;
