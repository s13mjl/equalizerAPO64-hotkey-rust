import re, glob, xml.etree.ElementTree as ET

srcs = set()
for fn in glob.glob('crates/editor/assets/translations/Editor_*.ts'):
    for ctx in ET.parse(fn).getroot().findall('context'):
        for msg in ctx.findall('message'):
            s = msg.findtext('source')
            tr = msg.find('translation')
            if s and tr is not None and (tr.text or '') and tr.get('type') != 'unfinished':
                srcs.add(s)

patterns = [
    r'ui\.label\(\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.button\(\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.heading\(\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.checkbox\([^,]+,\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.radio\([^,]+,\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.selectable_label\([^,]+,\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.selectable_value\([^,]+,[^,]+,\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.menu_button\(\s*"((?:[^"\\]|\\.)*)"',
    r'menu_button\(ui,\s*"((?:[^"\\]|\\.)*)"',
    r'egui::Window::new\(\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.colored_label\([^,]+,\s*"((?:[^"\\]|\\.)*)"',
    r'Button::new\(\s*"((?:[^"\\]|\\.)*)"',
    r'\.on_hover_text\(\s*"((?:[^"\\]|\\.)*)"',
    r'\.selected_text\(\s*"((?:[^"\\]|\\.)*)"',
    r'ui\.small_button\(\s*"((?:[^"\\]|\\.)*)"',
]

def unescape(s):
    return s.replace('\\"', '"').replace('\\n', '\n').replace('\\\\', '\\').replace("\\'", "'")

found = []
for fn in glob.glob('crates/editor/src/*.rs'):
    text = open(fn, encoding='utf-8').read()
    for pat in patterns:
        for m in re.finditer(pat, text):
            found.append((fn, unescape(m.group(1))))

uniq = sorted(set(s for _, s in found), key=str.lower)
matched = [s for s in uniq if s in srcs]
unmatched = [s for s in uniq if s not in srcs]
print('unique UI literals', len(uniq), 'matched', len(matched), 'unmatched', len(unmatched))
print('--- matched ---')
for s in matched:
    print(repr(s))
print('--- unmatched ---')
for s in unmatched:
    print(repr(s))
