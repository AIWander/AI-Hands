# Example: Scrape a Web Page

Follow the escalation ladder — start cheap, only launch Chrome if you have to.

## Rung 1: Static HTTP fetch (no browser)

```
browser_http_scrape(url: "https://news.ycombinator.com")
→ Returns parsed text content. Done in ~50ms.
```

## Rung 2: JS-rendered page (still no Chrome)

```
browser_smart_browse(url: "https://example.com/spa-page")
→ Renders JS via jsdom, returns content. No Chrome process.
```

## Rung 3: Full extraction with Chrome

```
browser_launch(headless: true)
browser_navigate(url: "https://example.com/complex-page")
browser_extract_content()
→ Clean text/markdown, ads and nav stripped out.
browser_close()
```

## Rung 4: Interactive scrape with a11y refs

```
browser_launch()
browser_navigate(url: "https://example.com/dashboard")
browser_a11y_snapshot()
→ See all interactive elements with refs like ref_0, ref_1...

browser_click(a11y_ref: "ref_12")  # click "Export" button
browser_wait_for(selector: ".download-link", visible: true)
browser_get_text(selector: ".download-link")
browser_close()
```

The point: most scraping tasks never need to go past Rung 1 or 2.
