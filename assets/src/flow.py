#!/usr/bin/env python3
"""Builds assets/flow.svg and assets/flow.ko.svg, the animated "how it works" loop.

The SVG animates with plain CSS so it also plays inside a README <img>. Run
`python3 assets/src/flow.py` from the repository root after changing it.
"""

from pathlib import Path

T = 13.0  # loop length in seconds
OUT = 12.4  # everything that appeared during the loop fades out here

STRINGS = {
    "en": {
        "lang": "en",
        "title": "How itda works: compile a site once, let the agent read, approve each change",
        "pills": ["1  Compile once", "2  Agent reads freely", "3  You approve changes"],
        "captions": ["WEBSITE", "ITDA", "AI AGENT"],
        "board": "Notices",
        "write": "Write",
        "rows": ["Maintenance: mail server", "Holiday schedule", "Maintenance: VPN"],
        "new_row": "Server maintenance",
        "compiled": "✓ 2 actions compiled",
        "audit": "✓ audit log chain verified",
        "agent": "AI agent",
        "ask_read": "Any maintenance notices?",
        "read_ok": "✓ 2 found · read ran now",
        "ask_write": "Post a server notice",
        "waiting": "⏸ approval_required",
        "write_ok": "✓ posted once · logged",
        "toast": "Approve this change?",
        "reject": "Reject",
        "approve": "Approve",
        "approved": "Approved once",
    },
    "ko": {
        "lang": "ko",
        "title": "itda 동작 방식: 사이트를 한 번 컴파일하고, 읽기는 바로, 변경은 승인 후",
        "pills": ["1  한 번 컴파일", "2  읽기는 바로 실행", "3  변경은 내가 승인"],
        "captions": ["웹사이트", "ITDA", "AI 에이전트"],
        "board": "공지사항",
        "write": "글쓰기",
        "rows": ["점검: 메일 서버", "휴무 일정", "점검: VPN"],
        "new_row": "서버 점검 안내",
        "compiled": "✓ 액션 2개 컴파일됨",
        "audit": "✓ 감사 로그 체인 검증됨",
        "agent": "AI 에이전트",
        "ask_read": "점검 공지 있어?",
        "read_ok": "✓ 2건 · 읽기는 바로 실행",
        "ask_write": "서버 점검 공지 올려줘",
        "waiting": "⏸ approval_required",
        "write_ok": "✓ 한 번만 게시 · 기록됨",
        "toast": "이 변경을 승인할까요?",
        "reject": "거절",
        "approve": "승인",
        "approved": "한 번 승인됨",
    },
}

SANS = ("-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, Helvetica, Arial, "
        "'Apple SD Gothic Neo', 'Malgun Gothic', 'Noto Sans KR', sans-serif")
MONO = ("ui-monospace, SFMono-Regular, Menlo, Consolas, 'JetBrains Mono', "
        "'D2Coding', 'Noto Sans Mono CJK KR', monospace")


def width(text, size):
    """Rough rendered width, enough to size bubbles around their text."""
    return sum(size * (1.0 if ord(c) >= 0x1100 else 0.56) for c in text)


def esc(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


class Anim:
    """Collects one CSS keyframe animation per animated element."""

    def __init__(self):
        self.rules = []
        self.n = 0

    def add(self, frames, ease=True):
        self.n += 1
        name = f"k{self.n}"
        frames = sorted(frames, key=lambda f: f[0])
        if frames[0][0] > 0:
            frames.insert(0, (0, frames[0][1]))
        if frames[-1][0] < T:
            frames.append((T, frames[-1][1]))
        body = " ".join(f"{t / T * 100:.2f}%{{{css}}}" for t, css in frames)
        timing = "cubic-bezier(.4,0,.2,1)" if ease else "linear"
        self.rules.append(f"@keyframes {name}{{{body}}}")
        self.rules.append(f".{name}{{animation:{name} {T}s {timing} infinite}}")
        return name

    def show(self, t_in, t_out=OUT, rise=6):
        """Fade in (rising a little) at t_in and out at t_out."""
        hidden = f"opacity:0;transform:translateY({rise}px)"
        shown = "opacity:1;transform:translateY(0)"
        gone = "opacity:0;transform:translateY(0)"
        return self.add([(t_in, hidden), (t_in + 0.35, shown), (t_out, shown), (t_out + 0.3, gone)])

    def packet(self, dx, t0, t1):
        """Moves a dot dx pixels between t0 and t1, visible only while moving."""
        return self.add([
            (t0 - 0.01, "opacity:0;transform:translateX(0)"),
            (t0, "opacity:1;transform:translateX(0)"),
            (t1, f"opacity:1;transform:translateX({dx}px)"),
            (t1 + 0.12, f"opacity:0;transform:translateX({dx}px)"),
        ])

    def pulse(self, t0):
        return self.add([
            (t0, "opacity:0;transform:scale(.85)"),
            (t0 + 0.15, "opacity:.7;transform:scale(.95)"),
            (t0 + 0.8, "opacity:0;transform:scale(1.25)"),
        ])


def build(s):
    a = Anim()
    out = []
    w = out.append

    # Header pills: a dim base, and a bright copy shown during its phase.
    pill_w, gap = 186, 12
    x0 = 480 - (3 * pill_w + 2 * gap) / 2
    phases = [(0.0, 3.6), (3.6, 7.0), (7.0, OUT)]
    for i, label in enumerate(s["pills"]):
        x = x0 + i * (pill_w + gap)
        w(f'<rect x="{x}" y="16" width="{pill_w}" height="30" rx="15" fill="#10131A" stroke="#242A38"/>')
        w(f'<text x="{x + pill_w / 2}" y="35.5" text-anchor="middle" class="sans" font-size="12.5" fill="#6B7385">{esc(label)}</text>')
        t_in, t_out = phases[i]
        k = a.add([(t_in, "opacity:0"), (t_in + 0.3, "opacity:1"), (t_out, "opacity:1"), (t_out + 0.3, "opacity:0")]) if i else \
            a.add([(0, "opacity:1"), (t_out, "opacity:1"), (t_out + 0.3, "opacity:0"), (T - 0.3, "opacity:0"), (T, "opacity:1")])
        k += " still" if i == 2 else ""
        w(f'<g class="{k}"><rect x="{x}" y="16" width="{pill_w}" height="30" rx="15" fill="#2F5BEA" fill-opacity=".18" stroke="#6E8BFF" stroke-opacity=".7"/>'
          f'<text x="{x + pill_w / 2}" y="35.5" text-anchor="middle" class="sans" font-size="12.5" font-weight="600" fill="#ECEEF3">{esc(label)}</text></g>')

    # Column captions.
    caps = s["captions"]
    w(f'<text x="24" y="76" class="mono cap">{esc(caps[0])}</text>')
    w(f'<text x="480" y="76" text-anchor="middle" class="mono cap">{esc(caps[1])}</text>')
    w(f'<text x="664" y="76" class="mono cap">{esc(caps[2])}</text>')

    # Links between the three columns.
    for x1, x2 in ((296, 432), (528, 664)):
        w(f'<line x1="{x1}" y1="144" x2="{x2}" y2="144" stroke="#2F3647" stroke-width="1.5" stroke-dasharray="3 5"/>')

    # Website card.
    w('<rect x="24" y="86" width="272" height="262" rx="14" fill="#141823" stroke="#242A38"/>')
    for cx in (42, 55, 68):
        w(f'<circle cx="{cx}" cy="104" r="4" fill="#2B3141"/>')
    w('<rect x="84" y="96" width="198" height="17" rx="8.5" fill="#0B0D12"/>')
    w('<text x="96" y="108.5" class="mono" font-size="10" fill="#6B7385">intranet.example/board</text>')
    w(f'<text x="40" y="143" class="sans" font-size="14" font-weight="600" fill="#ECEEF3">{esc(s["board"])}</text>')
    bw = width(s["write"], 11) + 20
    w(f'<rect x="{280 - bw}" y="128" width="{bw}" height="21" rx="6" fill="#2F5BEA" fill-opacity=".85"/>')
    w(f'<text x="{280 - bw / 2}" y="142.5" text-anchor="middle" class="sans" font-size="11" font-weight="600" fill="#fff">{esc(s["write"])}</text>')
    for i, row in enumerate(s["rows"]):
        y = 160 + i * 34
        w(f'<rect x="40" y="{y}" width="240" height="26" rx="6" fill="#10131A" stroke="#242A38"/>')
        w(f'<text x="52" y="{y + 17}" class="mono" font-size="10" fill="#6B7385">{12 + i}</text>')
        w(f'<text x="76" y="{y + 17}" class="sans" font-size="11.5" fill="#9AA3B5">{esc(row)}</text>')
    # The row the approved change adds.
    k = a.show(10.25) + " still"
    w(f'<g class="{k}"><rect x="40" y="262" width="240" height="26" rx="6" fill="#3DD68C" fill-opacity=".1" stroke="#3DD68C" stroke-opacity=".55"/>'
      f'<text x="52" y="279" class="mono" font-size="10" fill="#3DD68C">15</text>'
      f'<text x="76" y="279" class="sans" font-size="11.5" fill="#ECEEF3">{esc(s["new_row"])}</text></g>')
    # Compile: a highlight sweeps over the rows.
    k = a.add([
        (0.4, "opacity:0;transform:translateY(0)"),
        (0.6, "opacity:1;transform:translateY(0)"),
        (1.1, "opacity:1;transform:translateY(34px)"),
        (1.6, "opacity:1;transform:translateY(68px)"),
        (2.0, "opacity:0;transform:translateY(68px)"),
    ])
    w(f'<rect class="{k}" x="40" y="160" width="240" height="26" rx="6" fill="#2F5BEA" fill-opacity=".16" stroke="#6E8BFF" stroke-opacity=".8"/>')
    # Read: the two matching rows light up.
    k = a.add([(5.2, "opacity:0"), (5.4, "opacity:1"), (6.5, "opacity:1"), (6.9, "opacity:0")])
    w(f'<g class="{k}" fill="none" stroke="#3DD68C" stroke-opacity=".8">'
      '<rect x="40" y="160" width="240" height="26" rx="6"/><rect x="40" y="228" width="240" height="26" rx="6"/></g>')
    w('<text x="40" y="334" class="mono" font-size="10" fill="#6B7385">HTML · OpenAPI · HAR</text>')

    # itda core: the logo mark, with pulses when work passes through it.
    for t0, color in ((2.0, "#6E8BFF"), (4.85, "#3DD68C"), (9.75, "#3DD68C")):
        k = a.pulse(t0)
        w(f'<circle class="{k} origin" cx="480" cy="144" r="44" fill="none" stroke="{color}" stroke-width="2"/>')
    w('<g transform="translate(444 108) scale(.75)">'
      '<rect x="2" y="2" width="92" height="92" rx="24" fill="url(#m-bg)"/>'
      '<rect x="2.5" y="2.5" width="91" height="91" rx="23.5" fill="none" stroke="#fff" stroke-opacity=".1"/>'
      '<path d="M29 31 L46 48 L29 65" fill="none" stroke="#F4F5F8" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"/>'
      '<circle cx="63" cy="63" r="7.5" fill="url(#m-dot)"/></g>')
    k = a.add([(8.15, "opacity:0"), (8.35, "opacity:1"), (9.6, "opacity:1"), (9.8, "opacity:0")])
    w(f'<g class="{k}"><circle cx="480" cy="144" r="44" fill="#F5B041" fill-opacity=".08" stroke="#F5B041" stroke-width="2" stroke-dasharray="5 5"/>'
      '<rect x="502" y="160" width="22" height="22" rx="7" fill="#F5B041"/>'
      '<path d="M508.5 170v-2.5a4.5 4.5 0 0 1 9 0v2.5" fill="none" stroke="#0B0D12" stroke-width="1.8"/>'
      '<rect x="507" y="170" width="12" height="8" rx="2" fill="#0B0D12"/></g>')

    # Compiled definition.
    w('<rect x="372" y="196" width="216" height="152" rx="12" fill="#10131A" stroke="#242A38"/>')
    w('<text x="386" y="217" class="mono" font-size="10.5" fill="#6B7385">board/spec.yaml</text>')
    chips = (
        (232, "list-notices", "READ", "#3DD68C", 2.5),
        (266, "create-notice", "CHANGE", "#F5B041", 2.9),
    )
    for y, name, badge, color, t_in in chips:
        k = a.show(t_in) + " still"
        bw = len(badge) * 6.4 + 14
        w(f'<g class="{k}"><rect x="384" y="{y}" width="192" height="26" rx="7" fill="#141823" stroke="#2F3647"/>'
          f'<text x="396" y="{y + 17}" class="mono" font-size="11.5" fill="#ECEEF3">{name}</text>'
          f'<rect x="{568 - bw}" y="{y + 5}" width="{bw}" height="16" rx="8" fill="{color}" fill-opacity=".14"/>'
          f'<text x="{568 - bw / 2}" y="{y + 16.5}" text-anchor="middle" class="mono" font-size="9.5" font-weight="600" fill="{color}">{badge}</text></g>')
    for y, color, t0, t1 in ((232, "#3DD68C", 4.75, 6.2), (266, "#F5B041", 7.9, 10.3)):
        k = a.add([(t0, "opacity:0"), (t0 + 0.2, "opacity:1"), (t1, "opacity:1"), (t1 + 0.3, "opacity:0")])
        w(f'<rect class="{k}" x="384" y="{y}" width="192" height="26" rx="7" fill="none" stroke="{color}" stroke-width="1.5"/>')
    k = a.show(3.2, 10.9)
    w(f'<text class="{k}" x="386" y="322" font-size="10.5" fill="#3DD68C">{esc(s["compiled"])}</text>')
    k = a.show(11.0) + " still"
    w(f'<text class="{k}" x="386" y="322" font-size="10.5" fill="#6E8BFF">{esc(s["audit"])}</text>')

    # Agent card.
    w('<rect x="664" y="86" width="272" height="262" rx="14" fill="#141823" stroke="#242A38"/>')
    w('<path d="M688 100l2.6 6.4 6.4 2.6-6.4 2.6-2.6 6.4-2.6-6.4-6.4-2.6 6.4-2.6z" fill="#6E8BFF"/>')
    w(f'<text x="704" y="113" class="sans" font-size="13" font-weight="600" fill="#ECEEF3">{esc(s["agent"])}</text>')
    w('<rect x="884" y="100" width="36" height="17" rx="8.5" fill="none" stroke="#2F3647"/>')
    w('<text x="902" y="112" text-anchor="middle" class="mono" font-size="9.5" fill="#9AA3B5">MCP</text>')

    def ask(text, y, t_in):
        k = a.show(t_in) + " still"
        bw = width(text, 11.5) + 24
        w(f'<g class="{k}"><rect x="{920 - bw}" y="{y}" width="{bw}" height="26" rx="10" fill="#2F5BEA"/>'
          f'<text x="{920 - 12}" y="{y + 17}" text-anchor="end" class="sans" font-size="11.5" fill="#fff">{esc(text)}</text></g>')

    def call(text, y, t_in):
        k = a.show(t_in) + " still"
        w(f'<text class="{k}" x="680" y="{y}" font-size="11" fill="#6E8BFF">→ {esc(text)}</text>')

    def result(text, y, color, t_in, t_out=OUT, still=True):
        k = a.show(t_in, t_out) + (" still" if still else "")
        bw = width(text, 11) + 22
        w(f'<g class="{k}"><rect x="680" y="{y}" width="{bw}" height="26" rx="8" fill="{color}" fill-opacity=".1" stroke="{color}" stroke-opacity=".45"/>'
          f'<text x="691" y="{y + 17}" class="mono" font-size="11" fill="{color}">{esc(text)}</text></g>')

    ask(s["ask_read"], 128, 3.8)
    call("board.list-notices", 176, 4.25)
    result(s["read_ok"], 186, "#3DD68C", 6.5)
    ask(s["ask_write"], 224, 7.2)
    call("board.create-notice", 272, 7.6)
    result(s["waiting"], 284, "#F5B041", 8.3, 10.75, still=False)
    result(s["write_ok"], 284, "#3DD68C", 11.0)

    # Packets moving between columns.
    def dot(x, y, dx, t0, t1, color):
        k = a.packet(dx, t0, t1)
        w(f'<circle class="{k}" cx="{x}" cy="{y}" r="4.5" fill="{color}" filter="url(#glow)"/>')

    for i in range(3):
        dot(296, 144, 136, 1.0 + i * 0.25, 1.7 + i * 0.25, "#6E8BFF")
    dot(664, 144, -136, 4.35, 4.8, "#6E8BFF")
    dot(432, 144, -136, 4.85, 5.25, "#6E8BFF")
    dot(296, 144, 136, 5.6, 6.0, "#3DD68C")
    dot(528, 144, 136, 6.05, 6.45, "#3DD68C")
    dot(664, 144, -136, 7.7, 8.15, "#F5B041")
    dot(432, 144, -136, 9.8, 10.2, "#3DD68C")
    dot(296, 144, 136, 10.3, 10.6, "#3DD68C")
    dot(528, 144, 136, 10.6, 10.9, "#3DD68C")

    # Approval prompt shown to the person, and the click that approves it.
    k = a.add([
        (8.55, "opacity:0;transform:translateY(16px)"),
        (8.95, "opacity:1;transform:translateY(0)"),
        (9.95, "opacity:1;transform:translateY(0)"),
        (10.25, "opacity:0;transform:translateY(8px)"),
    ])
    rb, ab = width(s["reject"], 11.5) + 26, max(width(s["approve"], 11.5), width(s["approved"], 11.5)) + 26
    w(f'<g class="{k}"><rect x="290" y="334" width="380" height="94" rx="14" fill="#1A1F2C" stroke="#F5B041" stroke-opacity=".55" filter="url(#shadow)"/>'
      '<rect x="306" y="350" width="24" height="24" rx="7" fill="#F5B041" fill-opacity=".16"/>'
      '<path d="M313.5 361v-2.5a4.5 4.5 0 0 1 9 0v2.5" fill="none" stroke="#F5B041" stroke-width="1.8"/>'
      '<rect x="312" y="361" width="12" height="8" rx="2" fill="#F5B041"/>'
      f'<text x="342" y="366" class="sans" font-size="13" font-weight="600" fill="#ECEEF3">{esc(s["toast"])}</text>'
      f'<text x="342" y="386" class="mono" font-size="10.5" fill="#9AA3B5">board.create-notice  title="{esc(s["new_row"])}"</text>'
      f'<rect x="{654 - ab - 8 - rb}" y="396" width="{rb}" height="24" rx="7" fill="none" stroke="#2F3647"/>'
      f'<text x="{654 - ab - 8 - rb / 2}" y="412" text-anchor="middle" class="sans" font-size="11.5" fill="#9AA3B5">{esc(s["reject"])}</text>'
      f'<rect x="{654 - ab}" y="396" width="{ab}" height="24" rx="7" fill="#2F5BEA"/>'
      f'<text x="{654 - ab / 2}" y="412" text-anchor="middle" class="sans" font-size="11.5" font-weight="600" fill="#fff">{esc(s["approve"])}</text>')
    k2 = a.add([(9.5, "opacity:0"), (9.6, "opacity:1")])
    w(f'<g class="{k2}"><rect x="{654 - ab}" y="396" width="{ab}" height="24" rx="7" fill="#3DD68C"/>'
      f'<text x="{654 - ab / 2}" y="412" text-anchor="middle" class="sans" font-size="11.5" font-weight="600" fill="#0B0D12">{esc(s["approved"])}</text></g></g>')
    tx, ty = 654 - ab / 2 + 4, 410
    k = a.add([
        (8.9, f"opacity:0;transform:translate({tx + 90}px,{ty + 40}px) scale(1)"),
        (9.05, f"opacity:1;transform:translate({tx + 70}px,{ty + 30}px) scale(1)"),
        (9.4, f"opacity:1;transform:translate({tx}px,{ty}px) scale(1)"),
        (9.48, f"opacity:1;transform:translate({tx}px,{ty}px) scale(.82)"),
        (9.58, f"opacity:1;transform:translate({tx}px,{ty}px) scale(1)"),
        (9.95, f"opacity:1;transform:translate({tx}px,{ty}px) scale(1)"),
        (10.2, f"opacity:0;transform:translate({tx}px,{ty}px) scale(1)"),
    ])
    w(f'<g class="{k}"><path d="M0 0v15l4-3.6 2.6 6 2.6-1.1-2.6-5.9h5.4z" fill="#fff" stroke="#0B0D12" stroke-width="1.2" stroke-linejoin="round"/></g>')

    css = "\n".join(a.rules)
    head = f"""<svg xmlns="http://www.w3.org/2000/svg" width="960" height="440" viewBox="0 0 960 440" role="img" aria-label="{esc(s['title'])}" lang="{s['lang']}">
<title>{esc(s['title'])}</title>
<defs>
<linearGradient id="m-bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#232A3B"/><stop offset="1" stop-color="#0B0D12"/></linearGradient>
<linearGradient id="m-dot" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#6E8BFF"/><stop offset="1" stop-color="#2F5BEA"/></linearGradient>
<radialGradient id="bg-glow" cx=".5" cy="0" r=".7"><stop offset="0" stop-color="#2F5BEA" stop-opacity=".22"/><stop offset="1" stop-color="#2F5BEA" stop-opacity="0"/></radialGradient>
<filter id="glow" x="-200%" y="-200%" width="500%" height="500%"><feGaussianBlur stdDeviation="2.5" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter>
<filter id="shadow" x="-20%" y="-40%" width="140%" height="200%"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity=".6"/></filter>
</defs>
<style>
text{{font-family:{MONO}}}
.sans{{font-family:{SANS}}}
.cap{{font-size:10.5px;letter-spacing:.12em;fill:#6B7385}}
.origin{{transform-box:fill-box;transform-origin:center}}
[class^="k"]{{opacity:0}}
{css}
@media (prefers-reduced-motion: reduce){{[class^="k"]{{animation:none!important;transform:none!important}}.still{{opacity:1}}}}
</style>
<rect width="960" height="440" rx="20" fill="#0B0D12"/>
<rect width="960" height="440" rx="20" fill="url(#bg-glow)"/>
<rect x=".5" y=".5" width="959" height="439" rx="19.5" fill="none" stroke="#fff" stroke-opacity=".07"/>
"""
    return head + "\n".join(out) + "\n</svg>\n"


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[2]
    for lang, name in (("en", "flow.svg"), ("ko", "flow.ko.svg")):
        (root / "assets" / name).write_text(build(STRINGS[lang]), encoding="utf-8")
