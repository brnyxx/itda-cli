# A real first run: `itda`, one address, one Enter. Rendered with vhs
# (https://github.com/charmbracelet/vhs). Rebuild from the repository root:
#   DEMO_HOME=$(mktemp -d) vhs assets/src/setup.en.tape
# The run uses a throwaway HOME (with a Claude Desktop folder in it, so the
# app is found) and nothing on this machine changes.
Output assets/setup.gif

Require itda

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 17
Set Width 1320
Set Height 720
Set Padding 28
Set LineHeight 1.3
Set TypingSpeed 45ms
Set Theme { "name": "itda", "background": "#0B0D12", "foreground": "#ECEEF3", "cursor": "#6E8BFF", "black": "#242A38", "red": "#FF7AB6", "green": "#3DD68C", "yellow": "#F5B041", "blue": "#6E8BFF", "magenta": "#FF7AB6", "cyan": "#6E8BFF", "white": "#ECEEF3", "brightBlack": "#6B7385", "brightRed": "#FF7AB6", "brightGreen": "#3DD68C", "brightYellow": "#F5B041", "brightBlue": "#6E8BFF", "brightMagenta": "#FF7AB6", "brightCyan": "#6E8BFF", "brightWhite": "#FFFFFF" }

Hide
Type `export HOME="$DEMO_HOME" ITDA_HOME="$DEMO_HOME/itda" ITDA_SECRET_STORE=env ITDA_LANG=en PS1='$ '`
Enter
Type `mkdir -p "$HOME/Library/Application Support/Claude" && clear`
Enter
Show

Sleep 600ms
Type "itda"
Sleep 500ms
Enter
Wait+Screen@10s /(Address|주소):/
Sleep 900ms
Type "news.ycombinator.com"
Sleep 600ms
Enter
Wait+Screen@60s /\[Y\/n\]/
Sleep 1500ms
Enter
Wait+Screen@30s /(Done\.|끝났습니다\.)/
Sleep 5s
