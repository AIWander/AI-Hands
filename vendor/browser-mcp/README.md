# browser-mcp

**Rust MCP Browser Automation Server** - Full Playwright replacement in a single binary.

## Features

- **32 tools** for browser automation
- **Single binary** - no Python, no Node.js
- **chromiumoxide** - pure Rust CDP implementation
- **MCP protocol** - stdio JSON-RPC 2.0
- **4.8 MB** binary size

## Tools

### Core
| Tool | Description |
|------|-------------|
| `browser_launch` | Launch browser (headless/visible, profile support) |
| `browser_close` | Close browser |
| `browser_navigate` | Navigate to URL |
| `browser_click` | Click by selector or coordinates |
| `browser_type` | Type text into element |
| `browser_press` | Press keyboard key |
| `browser_screenshot` | Take screenshot (full page, element, quality) |
| `browser_wait_for` | Wait for element |

### Navigation
| Tool | Description |
|------|-------------|
| `browser_get_url` | Get current URL |
| `browser_back` | Go back |
| `browser_forward` | Go forward |
| `browser_reload` | Reload page |
| `browser_scroll` | Scroll (up/down/left/right) |

### DOM
| Tool | Description |
|------|-------------|
| `browser_get_html` | Get page/element HTML |
| `browser_get_text` | Get element text |
| `browser_evaluate` | Execute JavaScript |
| `browser_exists` | Check element exists |
| `browser_get_bounds` | Get element position/size |
| `browser_hover` | Hover element |
| `browser_focus` | Focus element |
| `browser_select` | Select dropdown option |

### Forms
| Tool | Description |
|------|-------------|
| `browser_get_forms` | Get all forms with fields |
| `browser_fill_form` | Fill form by field names |
| `browser_submit_form` | Submit form |

### Vision Support
| Tool | Description |
|------|-------------|
| `browser_get_clickables` | Get all clickable elements with coordinates |
| `browser_get_metrics` | Get page metrics (scroll, dimensions) |
| `browser_screenshot_burst` | Rapid screenshots for state tracking |

### Cookies
| Tool | Description |
|------|-------------|
| `browser_cookies` | Get/set/clear cookies |

### Other
| Tool | Description |
|------|-------------|
| `browser_status` | Get browser status |
| `browser_new_page` | Open new tab |
| `browser_inject_script` | Inject JavaScript |
| `browser_wait_idle` | Wait for network idle |

## Usage

### Claude Desktop Config
```json
{
  "mcpServers": {
    "browser": {
      "command": "C:\\rust-mcp\\target\\release\\browser-mcp.exe"
    }
  }
}
```

### Profile Mode (Keep Logins)
```
browser_launch(headless=false, profile_path="C:\\Users\\user\\ChromeAutomation")
```

First setup:
1. Create folder: `mkdir C:\Users\user\ChromeAutomation`
2. Launch Chrome: `chrome.exe --user-data-dir="C:\Users\user\ChromeAutomation"`
3. Log into sites
4. Close Chrome
5. Profile ready for automation

## Build

```bash
cd C:\rust-mcp\browser-mcp
cargo build --release
```

Binary: `C:\rust-mcp\target\release\browser-mcp.exe` (4.8 MB)

## vs Python Playwright MCP

| Aspect | browser-mcp | Python Playwright |
|--------|-------------|-------------------|
| Binary size | 4.8 MB | ~50 MB+ |
| Dependencies | None | Python, playwright |
| Startup | ~50ms | ~200ms |
| Memory | ~5 MB | ~50 MB |
| Tools | 32 | ~20 |
| Vision support | Built-in | External |

## Architecture

```
Claude → stdio JSON-RPC → browser-mcp.exe → chromiumoxide → Chrome CDP
```

- **chromiumoxide**: Pure Rust CDP client
- **MCP protocol**: 2024-11-05 spec
- **State**: Single browser instance per process
