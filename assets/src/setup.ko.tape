# A real run of itda setup on Hacker News, rendered with vhs
# (https://github.com/charmbracelet/vhs). Rebuild from the repository root:
#   DEMO_HOME=$(mktemp -d) vhs assets/src/setup.ko.tape
# The run uses a throwaway HOME so nothing on this machine changes.
Output assets/setup.ko.gif

Require itda

Set Shell "bash"
Set FontFamily "D2Coding"
Set FontSize 17
Set Width 1400
Set Height 600
Set Padding 28
Set LineHeight 1.3
Set TypingSpeed 45ms
Set Theme { "name": "itda", "background": "#0B0D12", "foreground": "#ECEEF3", "cursor": "#6E8BFF", "black": "#242A38", "red": "#FF7AB6", "green": "#3DD68C", "yellow": "#F5B041", "blue": "#6E8BFF", "magenta": "#FF7AB6", "cyan": "#6E8BFF", "white": "#ECEEF3", "brightBlack": "#6B7385", "brightRed": "#FF7AB6", "brightGreen": "#3DD68C", "brightYellow": "#F5B041", "brightBlue": "#6E8BFF", "brightMagenta": "#FF7AB6", "brightCyan": "#6E8BFF", "brightWhite": "#FFFFFF" }

Hide
Type `export HOME="$DEMO_HOME" ITDA_HOME="$DEMO_HOME/itda" ITDA_SECRET_STORE=env ITDA_LANG=ko PS1='$ '`
Enter
Type "clear"
Enter
Show

Sleep 600ms
Type "itda setup https://news.ycombinator.com/"
Sleep 700ms
Enter
Wait+Screen@60s /\[y\/N\]/
Sleep 1200ms
Enter
Wait+Screen@60s /\[0\] (later|나중에):/
Sleep 1500ms
Type "1"
Sleep 600ms
Enter
Wait+Screen@30s /(Done\.|끝났습니다\.)/
Sleep 5s
