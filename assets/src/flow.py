#!/usr/bin/env python3
"""Builds assets/flow.svg and assets/flow.ko.svg, the animated "how it works" loop.

The loop follows one ordinary website with a sign-in page and a write form:
you record it once in your browser, itda drafts commands from the recording,
and an agent calls them, with each change waiting for your approval.

The SVG animates with plain CSS so it also plays inside a README <img>. Run
`python3 assets/src/flow.py` from the repository root after changing it.
"""

from pathlib import Path

T = 15.6  # loop length in seconds
OUT = 15.0  # everything that appeared during the loop fades out here

STRINGS = {
    "en": {
        "lang": "en",
        "title": "How itda works: record a website once in your browser, itda drafts commands, "
                 "the agent calls them and every change waits for your approval",
        "pills": ["1  Record it once", "2  itda drafts commands", "3  Agent calls, you approve"],
        "captions": ["YOUR WEBSITE", "ITDA", "AI AGENT"],
        "sign_in": "Sign in",
        "user": "kim",
        "recording": "⏺ recording board.har",
        "recorded": "board.har saved · values dropped",
        "board": "Notices",
        "write": "Write",
        "rows": ["Maintenance: mail server", "Holiday schedule", "Maintenance: VPN"],
        "new_row": "Server maintenance",
        "preflight": "↳ fresh CSRF token per post",
        "drafted": "✓ 2 commands drafted",
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
        "title": "itda 동작 방식: 브라우저에서 웹사이트를 한 번 녹화하면 itda가 명령을 만들고, "
                 "에이전트가 호출하며, 변경은 모두 내 승인을 기다립니다",
        "pills": ["1  브라우저에서 한 번 녹화", "2  itda가 명령으로 변환", "3  에이전트 호출, 변경은 승인"],
        "captions": ["내 웹사이트", "ITDA", "AI 에이전트"],
        "sign_in": "로그인",
        "user": "kim",
        "recording": "⏺ board.har 녹화 중",
        "recorded": "board.har 저장 · 값은 버림",
        "board": "공지사항",
        "write": "글쓰기",
        "rows": ["점검: 메일 서버", "휴무 일정", "점검: VPN"],
        "new_row": "서버 점검 안내",
        "preflight": "↳ 게시마다 새 CSRF 토큰",
        "drafted": "✓ 명령 2개 생성됨",
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

    def loop_from_start(t_out):
        """Visible when the loop starts, gone at t_out, back just before it repeats."""
        return a.add([(0, "opacity:1"), (t_out, "opacity:1"), (t_out + 0.25, "opacity:0"),
                      (T - 0.3, "opacity:0"), (T, "opacity:1")])

    def cursor(points, t_click, t_out):
        """A pointer that glides through points, clicks at t_click, then fades."""
        (t0, x0, y0), *rest = points
        frames = [(t0, f"opacity:0;transform:translate({x0}px,{y0}px) scale(1)")]
        for t, x, y in rest:
            frames.append((t, f"opacity:1;transform:translate({x}px,{y}px) scale(1)"))
        x, y = rest[-1][1], rest[-1][2]
        frames += [
            (t_click, f"opacity:1;transform:translate({x}px,{y}px) scale(.82)"),
            (t_click + 0.1, f"opacity:1;transform:translate({x}px,{y}px) scale(1)"),
            (t_out, f"opacity:1;transform:translate({x}px,{y}px) scale(1)"),
            (t_out + 0.25, f"opacity:0;transform:translate({x}px,{y}px) scale(1)"),
        ]
        k = a.add(frames)
        w(f'<g class="{k}"><path d="M0 0v15l4-3.6 2.6 6 2.6-1.1-2.6-5.9h5.4z" fill="#fff" stroke="#0B0D12" stroke-width="1.2" stroke-linejoin="round"/></g>')

    # Header pills: a dim base, and a bright copy shown during its phase.
    pill_w, gap = 214, 12
    x0 = 480 - (3 * pill_w + 2 * gap) / 2
    phases = [(0.0, 4.2), (4.2, 6.6), (6.6, OUT)]
    for i, label in enumerate(s["pills"]):
        x = x0 + i * (pill_w + gap)
        w(f'<rect x="{x}" y="16" width="{pill_w}" height="30" rx="15" fill="#10131A" stroke="#242A38"/>')
        w(f'<text x="{x + pill_w / 2}" y="35.5" text-anchor="middle" class="sans" font-size="12.5" fill="#6B7385">{esc(label)}</text>')
        t_in, t_out = phases[i]
        k = a.add([(t_in, "opacity:0"), (t_in + 0.3, "opacity:1"), (t_out, "opacity:1"), (t_out + 0.3, "opacity:0")]) if i \
            else loop_from_start(t_out)
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

    # Website card: a browser window with an address bar and a recording badge.
    w('<rect x="24" y="86" width="272" height="262" rx="14" fill="#141823" stroke="#242A38"/>')
    for cx in (42, 55, 68):
        w(f'<circle cx="{cx}" cy="104" r="4" fill="#2B3141"/>')
    w('<rect x="84" y="96" width="150" height="17" rx="8.5" fill="#0B0D12"/>')
    k = loop_from_start(1.65)
    w(f'<text class="{k}" x="96" y="108.5" font-size="10" fill="#6B7385">intranet.example/login</text>')
    k = a.show(1.75, rise=0) + " still"
    w(f'<text class="{k}" x="96" y="108.5" font-size="10" fill="#6B7385">intranet.example/board</text>')
    k = a.add([(0.2, "opacity:0"), (0.4, "opacity:1"), (3.5, "opacity:1"), (3.7, "opacity:0")])
    w(f'<g class="{k}"><rect x="242" y="96" width="40" height="17" rx="8.5" fill="#FF5C5C" fill-opacity=".14"/>'
      '<circle cx="253" cy="104.5" r="3.5" fill="#FF5C5C"/>'
      '<text x="260" y="108.5" font-size="9.5" font-weight="600" fill="#FF7A7A">REC</text></g>')

    # Sign-in page, shown while the recording starts.
    k = loop_from_start(1.65)
    w(f'<g class="{k}"><text x="40" y="146" class="sans" font-size="15" font-weight="600" fill="#ECEEF3">{esc(s["sign_in"])}</text>'
      '<rect x="40" y="162" width="240" height="28" rx="7" fill="#0B0D12" stroke="#2F3647"/>'
      '<rect x="40" y="200" width="240" height="28" rx="7" fill="#0B0D12" stroke="#2F3647"/>'
      '<rect x="40" y="244" width="240" height="30" rx="8" fill="#2F5BEA"/>'
      f'<text x="160" y="263.5" text-anchor="middle" class="sans" font-size="12" font-weight="600" fill="#fff">{esc(s["sign_in"])}</text></g>')
    k = a.add([(0.45, "opacity:0"), (0.6, "opacity:1"), (1.65, "opacity:1"), (1.9, "opacity:0")])
    w(f'<text class="{k}" x="52" y="180" font-size="11.5" fill="#ECEEF3">{esc(s["user"])}</text>')
    k = a.add([(0.8, "opacity:0"), (0.95, "opacity:1"), (1.65, "opacity:1"), (1.9, "opacity:0")])
    w(f'<text class="{k}" x="52" y="219" font-size="11.5" letter-spacing="2" fill="#ECEEF3">••••••••</text>')
    cursor([(0.5, 230, 300), (0.75, 210, 280), (1.15, 168, 262)], 1.3, 1.5)

    # The notice board behind the sign-in.
    k = a.show(1.75, rise=0) + " still"
    bw = width(s["write"], 11) + 20
    rows = "".join(
        f'<rect x="40" y="{160 + i * 34}" width="240" height="26" rx="6" fill="#10131A" stroke="#242A38"/>'
        f'<text x="52" y="{177 + i * 34}" font-size="10" fill="#6B7385">{12 + i}</text>'
        f'<text x="76" y="{177 + i * 34}" class="sans" font-size="11.5" fill="#9AA3B5">{esc(row)}</text>'
        for i, row in enumerate(s["rows"]))
    w(f'<g class="{k}"><text x="40" y="143" class="sans" font-size="14" font-weight="600" fill="#ECEEF3">{esc(s["board"])}</text>'
      f'<rect x="{280 - bw}" y="128" width="{bw}" height="21" rx="6" fill="#2F5BEA" fill-opacity=".85"/>'
      f'<text x="{280 - bw / 2}" y="142.5" text-anchor="middle" class="sans" font-size="11" font-weight="600" fill="#fff">{esc(s["write"])}</text>'
      f'{rows}</g>')
    # Using the site while it records: browsing the rows, then opening the form.
    k = a.add([
        (2.0, "opacity:0;transform:translateY(0)"),
        (2.15, "opacity:1;transform:translateY(0)"),
        (2.5, "opacity:1;transform:translateY(34px)"),
        (2.85, "opacity:1;transform:translateY(68px)"),
        (3.05, "opacity:0;transform:translateY(68px)"),
    ])
    w(f'<rect class="{k}" x="40" y="160" width="240" height="26" rx="6" fill="#2F5BEA" fill-opacity=".16" stroke="#6E8BFF" stroke-opacity=".8"/>')
    cursor([(2.9, 200, 200), (3.05, 220, 170), (3.25, 252, 140)], 3.35, 3.55)
    # The row the approved change adds.
    k = a.show(13.25) + " still"
    w(f'<g class="{k}"><rect x="40" y="262" width="240" height="26" rx="6" fill="#3DD68C" fill-opacity=".1" stroke="#3DD68C" stroke-opacity=".55"/>'
      f'<text x="52" y="279" font-size="10" fill="#3DD68C">15</text>'
      f'<text x="76" y="279" class="sans" font-size="11.5" fill="#ECEEF3">{esc(s["new_row"])}</text></g>')
    # Read: the two matching rows light up.
    k = a.add([(8.2, "opacity:0"), (8.4, "opacity:1"), (9.5, "opacity:1"), (9.9, "opacity:0")])
    w(f'<g class="{k}" fill="none" stroke="#3DD68C" stroke-opacity=".8">'
      '<rect x="40" y="160" width="240" height="26" rx="6"/><rect x="40" y="228" width="240" height="26" rx="6"/></g>')
    k = a.add([(0.2, "opacity:0"), (0.4, "opacity:1"), (3.5, "opacity:1"), (3.7, "opacity:0")])
    w(f'<text class="{k}" x="40" y="334" font-size="10" fill="#FF7A7A">{esc(s["recording"])}</text>')
    k = a.show(3.75, rise=0) + " still"
    w(f'<text class="{k}" x="40" y="334" font-size="10" fill="#6B7385">{esc(s["recorded"])}</text>')

    # The recording travels to itda.
    k = a.add([
        (3.6, "opacity:0;transform:translateX(0)"),
        (3.75, "opacity:1;transform:translateX(0)"),
        (4.25, "opacity:1;transform:translateX(62px)"),
        (4.4, "opacity:0;transform:translateX(62px)"),
    ])
    w(f'<g class="{k}"><rect x="304" y="134" width="68" height="20" rx="6" fill="#141823" stroke="#6E8BFF" stroke-opacity=".8"/>'
      '<text x="338" y="148" text-anchor="middle" font-size="10" fill="#B4C3FF">board.har</text></g>')

    # itda core: the logo mark, with pulses when work passes through it.
    for t0, color in ((4.3, "#6E8BFF"), (7.85, "#3DD68C"), (12.75, "#3DD68C")):
        k = a.pulse(t0)
        w(f'<circle class="{k} origin" cx="480" cy="144" r="44" fill="none" stroke="{color}" stroke-width="2"/>')
    w('<g transform="translate(444 108) scale(.75)">'
      '<rect x="2" y="2" width="92" height="92" rx="24" fill="url(#m-bg)"/>'
      '<rect x="2.5" y="2.5" width="91" height="91" rx="23.5" fill="none" stroke="#fff" stroke-opacity=".1"/>'
      '<path d="M29 31 L46 48 L29 65" fill="none" stroke="#F4F5F8" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"/>'
      '<circle cx="63" cy="63" r="7.5" fill="url(#m-dot)"/></g>')
    k = a.add([(11.15, "opacity:0"), (11.35, "opacity:1"), (12.6, "opacity:1"), (12.8, "opacity:0")])
    w(f'<g class="{k}"><circle cx="480" cy="144" r="44" fill="#F5B041" fill-opacity=".08" stroke="#F5B041" stroke-width="2" stroke-dasharray="5 5"/>'
      '<rect x="502" y="160" width="22" height="22" rx="7" fill="#F5B041"/>'
      '<path d="M508.5 170v-2.5a4.5 4.5 0 0 1 9 0v2.5" fill="none" stroke="#0B0D12" stroke-width="1.8"/>'
      '<rect x="507" y="170" width="12" height="8" rx="2" fill="#0B0D12"/></g>')

    # Drafted definition.
    w('<rect x="372" y="196" width="216" height="152" rx="12" fill="#10131A" stroke="#242A38"/>')
    k = a.show(4.35, rise=0) + " still"
    w(f'<text class="{k}" x="386" y="218" font-size="10.5" fill="#9AA3B5"><tspan fill="#6E8BFF">$</tspan> itda draft har board.har</text>')
    chips = (
        (232, "list-notices", "READ", "#3DD68C", 4.9),
        (266, "create-notice", "CHANGE", "#F5B041", 5.3),
    )
    for y, name, badge, color, t_in in chips:
        k = a.show(t_in) + " still"
        bw = len(badge) * 6.4 + 14
        w(f'<g class="{k}"><rect x="384" y="{y}" width="192" height="26" rx="7" fill="#141823" stroke="#2F3647"/>'
          f'<text x="396" y="{y + 17}" font-size="11.5" fill="#ECEEF3">{name}</text>'
          f'<rect x="{568 - bw}" y="{y + 5}" width="{bw}" height="16" rx="8" fill="{color}" fill-opacity=".14"/>'
          f'<text x="{568 - bw / 2}" y="{y + 16.5}" text-anchor="middle" font-size="9.5" font-weight="600" fill="{color}">{badge}</text></g>')
    for y, color, t0, t1 in ((232, "#3DD68C", 7.75, 9.2), (266, "#F5B041", 10.9, 13.3)):
        k = a.add([(t0, "opacity:0"), (t0 + 0.2, "opacity:1"), (t1, "opacity:1"), (t1 + 0.3, "opacity:0")])
        w(f'<rect class="{k}" x="384" y="{y}" width="192" height="26" rx="7" fill="none" stroke="{color}" stroke-width="1.5"/>')
    k = a.show(5.7) + " still"
    w(f'<text class="{k}" x="396" y="308" font-size="10" fill="#6B7385">{esc(s["preflight"])}</text>')
    k = a.add([(12.75, "opacity:0"), (12.9, "opacity:1"), (13.3, "opacity:1"), (13.6, "opacity:0")])
    w(f'<text class="{k}" x="396" y="308" font-size="10" fill="#3DD68C">{esc(s["preflight"])}</text>')
    k = a.show(6.0, 13.8)
    w(f'<text class="{k}" x="386" y="332" font-size="10.5" fill="#3DD68C">{esc(s["drafted"])}</text>')
    k = a.show(14.0) + " still"
    w(f'<text class="{k}" x="386" y="332" font-size="10.5" fill="#6E8BFF">{esc(s["audit"])}</text>')

    # Agent card.
    w('<rect x="664" y="86" width="272" height="262" rx="14" fill="#141823" stroke="#242A38"/>')
    w('<path d="M688 100l2.6 6.4 6.4 2.6-6.4 2.6-2.6 6.4-2.6-6.4-6.4-2.6 6.4-2.6z" fill="#6E8BFF"/>')
    w(f'<text x="704" y="113" class="sans" font-size="13" font-weight="600" fill="#ECEEF3">{esc(s["agent"])}</text>')
    w('<rect x="854" y="100" width="66" height="17" rx="8.5" fill="none" stroke="#2F3647"/>')
    w('<text x="887" y="112" text-anchor="middle" font-size="9.5" fill="#9AA3B5">CLI · MCP</text>')

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
          f'<text x="691" y="{y + 17}" font-size="11" fill="{color}">{esc(text)}</text></g>')

    ask(s["ask_read"], 128, 6.8)
    call("board.list-notices", 176, 7.25)
    result(s["read_ok"], 186, "#3DD68C", 9.5)
    ask(s["ask_write"], 224, 10.2)
    call("board.create-notice", 272, 10.6)
    result(s["waiting"], 284, "#F5B041", 11.3, 13.75, still=False)
    result(s["write_ok"], 284, "#3DD68C", 14.0)

    # Requests moving between columns.
    def dot(x, y, dx, t0, t1, color):
        k = a.packet(dx, t0, t1)
        w(f'<circle class="{k}" cx="{x}" cy="{y}" r="4.5" fill="{color}" filter="url(#glow)"/>')

    dot(664, 144, -136, 7.35, 7.8, "#6E8BFF")
    dot(432, 144, -136, 7.85, 8.25, "#6E8BFF")
    dot(296, 144, 136, 8.6, 9.0, "#3DD68C")
    dot(528, 144, 136, 9.05, 9.45, "#3DD68C")
    dot(664, 144, -136, 10.7, 11.15, "#F5B041")
    dot(432, 144, -136, 12.8, 13.2, "#3DD68C")
    dot(296, 144, 136, 13.3, 13.6, "#3DD68C")
    dot(528, 144, 136, 13.6, 13.9, "#3DD68C")

    # Approval prompt shown to the person, and the click that approves it.
    k = a.add([
        (11.55, "opacity:0;transform:translateY(16px)"),
        (11.95, "opacity:1;transform:translateY(0)"),
        (12.95, "opacity:1;transform:translateY(0)"),
        (13.25, "opacity:0;transform:translateY(8px)"),
    ])
    rb, ab = width(s["reject"], 11.5) + 26, max(width(s["approve"], 11.5), width(s["approved"], 11.5)) + 26
    w(f'<g class="{k}"><rect x="290" y="334" width="380" height="94" rx="14" fill="#1A1F2C" stroke="#F5B041" stroke-opacity=".55" filter="url(#shadow)"/>'
      '<rect x="306" y="350" width="24" height="24" rx="7" fill="#F5B041" fill-opacity=".16"/>'
      '<path d="M313.5 361v-2.5a4.5 4.5 0 0 1 9 0v2.5" fill="none" stroke="#F5B041" stroke-width="1.8"/>'
      '<rect x="312" y="361" width="12" height="8" rx="2" fill="#F5B041"/>'
      f'<text x="342" y="366" class="sans" font-size="13" font-weight="600" fill="#ECEEF3">{esc(s["toast"])}</text>'
      f'<text x="342" y="386" font-size="10.5" fill="#9AA3B5">board.create-notice  title="{esc(s["new_row"])}"</text>'
      f'<rect x="{654 - ab - 8 - rb}" y="396" width="{rb}" height="24" rx="7" fill="none" stroke="#2F3647"/>'
      f'<text x="{654 - ab - 8 - rb / 2}" y="412" text-anchor="middle" class="sans" font-size="11.5" fill="#9AA3B5">{esc(s["reject"])}</text>'
      f'<rect x="{654 - ab}" y="396" width="{ab}" height="24" rx="7" fill="#2F5BEA"/>'
      f'<text x="{654 - ab / 2}" y="412" text-anchor="middle" class="sans" font-size="11.5" font-weight="600" fill="#fff">{esc(s["approve"])}</text>')
    k2 = a.add([(12.5, "opacity:0"), (12.6, "opacity:1")])
    w(f'<g class="{k2}"><rect x="{654 - ab}" y="396" width="{ab}" height="24" rx="7" fill="#3DD68C"/>'
      f'<text x="{654 - ab / 2}" y="412" text-anchor="middle" class="sans" font-size="11.5" font-weight="600" fill="#0B0D12">{esc(s["approved"])}</text></g></g>')
    tx, ty = 654 - ab / 2 + 4, 410
    cursor([(11.9, tx + 90, ty + 40), (12.05, tx + 70, ty + 30), (12.4, tx, ty)], 12.48, 12.95)

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
