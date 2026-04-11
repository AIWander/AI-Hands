# Example: Graduate a Browser Flow to Direct API Calls

The graduation pipeline: automate it in the browser once, capture the
underlying API, skip the browser next time.

## Step 1: Do the flow in the browser

```
browser_launch(profile_path: "C:/profiles/myapp")
browser_navigate(url: "https://app.example.com/login")
browser_fill_form(fields: [
  {"selector": "#email", "value": "user@example.com"},
  {"selector": "#password", "value": "hunter2"}
])
browser_click(a11y_ref: "ref_8")  # Login button
browser_navigate(url: "https://app.example.com/reports")
browser_click(a11y_ref: "ref_22")  # Export button
```

## Step 2: Capture all network traffic

```
browser_get_all_network()
→ Returns 47 HTTP requests the page made during the session.
```

## Step 3: Extract the real API endpoints

```
browser_learn_api()
→ Filters out analytics/tracking noise, returns structured endpoints:
  POST /api/v2/auth/login  (auth, returns token)
  GET  /api/v2/reports?range=30d  (data fetch)
  POST /api/v2/exports  (creates export job)
  GET  /api/v2/exports/{id}/download  (fetches file)
```

## Step 4: Future runs — no browser needed

```
browser_http_scrape(
  url: "https://app.example.com/api/v2/reports?range=30d",
  headers: {"Authorization": "Bearer <token>"}
)
→ 200ms instead of 5 seconds. No Chrome process.
```

## The payoff

First run: browser session, seconds of latency, Chrome overhead.
Every run after: direct HTTP, milliseconds, zero overhead.
