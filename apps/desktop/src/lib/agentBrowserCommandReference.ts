/** Condensed upstream command surface for Lux AI (full parity via BrowserInvoke). */
export const AGENT_BROWSER_COMMAND_REFERENCE = `
agent-browser command groups (use BrowserInvoke with args array; session is set by Lux):

Core: open, click, dblclick, fill, type, press, hover, select, check, uncheck, scroll, scrollintoview, drag, upload, screenshot, pdf, snapshot, eval, connect, close, batch, chat
Get: get text|html|value|attr|title|url|cdp-url|count|box|styles
State: is visible|enabled|checked
Find: find role|text|label|placeholder|alt|title|testid|first|last|nth
Wait: wait <selector|ms> (--text, --url, --load, --fn, --state)
Navigation: back, forward, reload, pushstate
Tabs: tab, tab new, tab close, window new
Frames: frame, frame main
Dialogs: dialog accept|dismiss|status
Mouse: mouse move|down|up|wheel
Clipboard: clipboard read|write|copy|paste
Settings: set viewport|device|geo|offline|headers|credentials|media
Cookies/Storage: cookies, storage local|session
Network: network route|unroute|requests|har
Debug: trace, profiler, console, errors, highlight, inspect, state, diff
Auth: auth save|login|list
React: react tree|inspect|renders|suspense (needs --enable react-devtools on open)
Vitals: vitals
Stream: stream enable|disable|status
Dashboard: dashboard start|stop|status
Setup: install, upgrade, doctor, skills
`.trim();