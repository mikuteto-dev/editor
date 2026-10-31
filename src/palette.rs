use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    component::{
        IndexPath, WindowExt,
        command::{Command, CommandItem, CommandState},
    },
    *,
};

pub fn open(
    items: Vec<CommandItem>,
    selected: Option<usize>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_confirm: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let state = cx.new(|cx| {
        let mut state = CommandState::new(window, cx);
        state.set_selected_index(selected.map(|row| IndexPath::default().row(row)), window, cx);
        state
    });
    let (on_select, on_confirm, on_close) = (Rc::new(on_select), Rc::new(on_confirm), Rc::new(on_close));
    let focused = Rc::new(Cell::new(false));

    window.open_dialog(cx, move |dialog, _, _| {
        let (state, items, focused) = (state.clone(), items.clone(), focused.clone());
        let (on_select, on_confirm, on_close) = (on_select.clone(), on_confirm.clone(), on_close.clone());
        dialog
            .close_button(false)
            .p_0()
            .on_close(move |_, window, cx| on_close(window, cx))
            .content(move |content, window, cx| {
                if !focused.replace(true) {
                    state.read(cx).focus_handle(cx).focus(window, cx);
                }
                let (on_select, on_confirm) = (on_select.clone(), on_confirm.clone());
                content.child(
                    Command::new(&state)
                        .bordered(false)
                        .max_h(px(400.))
                        .items(items.clone())
                        .on_select(move |ix, window, cx| on_select(ix.row, window, cx))
                        .on_confirm(move |ix, window, cx| {
                            on_confirm(ix.row, window, cx);
                            window.close_dialog(cx);
                        }),
                )
            })
    });
}
