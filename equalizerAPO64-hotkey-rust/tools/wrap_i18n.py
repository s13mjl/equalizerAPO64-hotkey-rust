import re
import pathlib

FILES = [
    "crates/editor/src/main_window.rs",
    "crates/editor/src/filter_table.rs",
    "crates/editor/src/guis.rs",
]
SKIP = {"", "*", "+", "=", "x"}


def literal_call(text, prefix, replacement_func):
    pattern = re.compile(re.escape(prefix) + r'\(\s*"([^"]*)"')
    return pattern.sub(replacement_func, text)


def wrap_one(text):
    count = [0]

    def repl(m):
        lit = m.group(1)
        if lit in SKIP:
            return m.group(0)
        count[0] += 1
        return m.group(0).replace('"' + lit + '"', 'crate::i18n::tr("' + lit + '")', 1)

    for func in [
        "ui.label",
        "ui.button",
        "ui.small_button",
        "ui.heading",
        "ui.link",
        "ui.menu_button",
        "egui::Window::new",
        "egui::Button::new",
        "egui::Label::new",
    ]:
        text = literal_call(text, func, repl)

    pattern = re.compile(r'menu_button\(ui,\s*"([^"]*)"')

    def menu_repl(m):
        if m.group(1) in SKIP:
            return m.group(0)
        return f'menu_button(ui, &crate::i18n::tr("{m.group(1)}")'

    text = pattern.sub(menu_repl, text)
    return text, count[0]


def wrap_two_arg(text, func):
    pattern = re.compile(re.escape(func) + r'\(([^,()]*),\s*"([^"]*)"')

    def repl(m):
        if m.group(2) in SKIP:
            return m.group(0)
        return f'{func}({m.group(1)}, crate::i18n::tr("{m.group(2)}")'

    return pattern.sub(repl, text)


def wrap_three_arg(text, func):
    pattern = re.compile(re.escape(func) + r'\(([^,()]*),\s*([^,()]+),\s*"([^"]*)"')

    def repl(m):
        if m.group(3) in SKIP:
            return m.group(0)
        return f'{func}({m.group(1)}, {m.group(2)}, crate::i18n::tr("{m.group(3)}")'

    return pattern.sub(repl, text)


for fn in FILES:
    p = pathlib.Path(fn)
    text = p.read_text(encoding="utf-8")
    original = text
    total = 0
    for func in ["ui.checkbox", "ui.radio", "ui.selectable_label", "ui.colored_label"]:
        new = wrap_two_arg(text, func)
        total += 1 if new != text else 0
        text = new
    for func in ["ui.selectable_value"]:
        new = wrap_three_arg(text, func)
        total += 1 if new != text else 0
        text = new
    text, n = wrap_one(text)
    total += n
    for suffix in [".on_hover_text", ".selected_text"]:
        pattern = re.compile(re.escape(suffix) + r'\(\s*"([^"]*)"')

        def repl(m):
            if m.group(1) in SKIP:
                return m.group(0)
            return f'{suffix}(crate::i18n::tr("{m.group(1)}")'

        new = pattern.sub(repl, text)
        total += 1 if new != text else 0
        text = new
    if text != original:
        p.write_text(text, encoding="utf-8")
    print(fn, "wrapped calls:", total)
