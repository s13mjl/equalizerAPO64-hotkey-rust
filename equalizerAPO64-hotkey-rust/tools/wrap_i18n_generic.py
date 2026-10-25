import re
import sys
import pathlib

SKIP = {"", "*", "+", "=", "x"}


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


def wrap_one(text):
    def repl(m):
        lit = m.group(1)
        if lit in SKIP:
            return m.group(0)
        return m.group(0).replace('"' + lit + '"', 'crate::i18n::tr("' + lit + '")', 1)

    for func in [
        "ui.label", "ui.button", "ui.small_button", "ui.heading", "ui.link",
        "ui.menu_button", "egui::Window::new", "egui::Button::new", "egui::Label::new",
    ]:
        text = re.compile(re.escape(func) + r'\(\s*"([^"]*)"').sub(repl, text)
    pattern = re.compile(r'menu_button\(ui,\s*"([^"]*)"')
    def menu_repl(m):
        if m.group(1) in SKIP:
            return m.group(0)
        return f'menu_button(ui, &crate::i18n::tr("{m.group(1)}")'
    text = pattern.sub(menu_repl, text)
    return text


for fn in sys.argv[1:]:
    p = pathlib.Path(fn)
    text = p.read_text(encoding="utf-8")
    original = text
    for func in ["ui.checkbox", "ui.radio", "ui.selectable_label", "ui.colored_label"]:
        text = wrap_two_arg(text, func)
    text = wrap_three_arg(text, "ui.selectable_value")
    text = wrap_one(text)
    for suffix in [".on_hover_text", ".selected_text"]:
        pattern = re.compile(re.escape(suffix) + r'\(\s*"([^"]*)"')
        def repl(m):
            if m.group(1) in SKIP:
                return m.group(0)
            return f'{suffix}(crate::i18n::tr("{m.group(1)}")'
        text = pattern.sub(repl, text)
    if text != original:
        p.write_text(text, encoding="utf-8")
        print(fn, "localized")
