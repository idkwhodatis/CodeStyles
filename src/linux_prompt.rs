//! Linux's GPUI fallback clips long disclosures. Keep its response/focus contract,
//! but constrain and wrap the content instead of changing the warning text.
use gpui_kit::{component::ActiveTheme, prelude::*, *};

pub fn init(cx: &mut App) {
    cx.set_prompt_builder(|_, message, detail, actions, handle, window, cx| {
        let view = cx.new(|cx| LinuxPrompt {
            message: message.to_owned(),
            detail: detail.map(str::to_owned),
            actions: actions.to_vec(),
            focus: cx.focus_handle(),
        });
        // GPUI owns one-shot completion, prompt dismissal and focus restoration.
        handle.with_view(view, window, cx)
    });
}

struct LinuxPrompt {
    message: String,
    detail: Option<String>,
    actions: Vec<PromptButton>,
    focus: FocusHandle,
}

fn prompt_bounds(width: f32, height: f32) -> (f32, f32) {
    ((width - 32.0).clamp(0.0, 640.0), (height - 32.0).max(0.0))
}

impl Render for LinuxPrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let (width, height) = prompt_bounds(viewport.width.into(), viewport.height.into());
        let panel = div()
            .id("linux-confirmation")
            .track_focus(&self.focus)
            .w(px(width))
            .max_h(px(height))
            .min_w_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded_lg()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .whitespace_normal()
                    .child(self.message.clone()),
            )
            .children(self.detail.clone().map(|detail| {
                div()
                    .id("linux-confirmation-detail")
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .max_h(px(height * 0.5))
                    .overflow_y_scroll()
                    .whitespace_normal()
                    .text_sm()
                    .child(detail)
            }))
            .children(self.actions.iter().enumerate().map(|(index, action)| {
                div()
                    .id(("linux-confirmation-answer", index))
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .whitespace_normal()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_md()
                    .p_2()
                    .cursor_pointer()
                    .text_sm()
                    .child(action.label().clone())
                    .on_click(cx.listener(move |_, _, _, cx| {
                        // Preserve the caller's exact answer order, including Cancel.
                        cx.emit(PromptResponse(index));
                        cx.stop_propagation();
                    }))
            }));
        div()
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .bg(rgba(0x00000099))
            .flex()
            .items_center()
            .justify_center()
            .child(panel)
    }
}

impl EventEmitter<PromptResponse> for LinuxPrompt {}
impl Focusable for LinuxPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::prompt_bounds;

    #[test]
    fn prompt_stays_inside_small_and_large_viewports() {
        for (width, height) in [(1180.0, 812.0), (320.0, 240.0), (40.0, 40.0), (0.0, 0.0)] {
            let (panel_width, panel_height) = prompt_bounds(width, height);
            assert!(panel_width >= 0.0 && panel_width <= width.min(640.0));
            assert!(panel_height >= 0.0 && panel_height <= height);
        }
        assert_eq!(prompt_bounds(1180.0, 812.0), (640.0, 780.0));
        assert_eq!(prompt_bounds(320.0, 240.0), (288.0, 208.0));
    }
}
