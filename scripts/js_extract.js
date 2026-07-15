// CPC Browser MCP - JS Extraction Engine
// Called by browser-mcp as: node js_extract.js <json_args>
// Args: {"url":"...","selector":"...","engine":"jsdom|linkedom","timeout":5000}

const args = JSON.parse(process.argv[2] || '{}');
const url = args.url;
const selector = args.selector || null;
const engine = args.engine || 'linkedom';
const timeout = args.timeout || 5000;

if (!url) {
  console.log(JSON.stringify({error: 'Missing url parameter'}));
  process.exit(1);
}

async function fetchHTML(url) {
  const resp = await fetch(url, {
    headers: {'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'},
    signal: AbortSignal.timeout(timeout)
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return await resp.text();
}

async function extractLinkedom(html) {
  const {parseHTML} = require('linkedom');
  const {document} = parseHTML(html);
  if (selector) {
    const el = document.querySelector(selector);
    return el ? el.textContent.trim() : '';
  }
  // Try common content selectors
  for (const sel of ['article', 'main', '[role="main"]', '.content', '#content', '.post-content', '.entry-content']) {
    const el = document.querySelector(sel);
    if (el && el.textContent.trim().length > 100) return el.textContent.trim();
  }
  // Fallback: body minus nav/header/footer/script/style
  const body = document.querySelector('body');
  if (!body) return document.documentElement.textContent.trim();
  for (const tag of ['script','style','nav','header','footer','aside']) {
    body.querySelectorAll(tag).forEach(el => el.remove());
  }
  return body.textContent.trim().replace(/\n{3,}/g, '\n\n');
}

async function extractJSDOM(html, url) {
  const {JSDOM} = require('jsdom');
  const dom = new JSDOM(html, {
    url: url,
    pretendToBeVisual: true
  });
  // Parse only. Executing untrusted page scripts in Node is intentionally disabled.
  // Use the attached browser DOM tools when live JavaScript execution is required.
  const doc = dom.window.document;
  if (selector) {
    const el = doc.querySelector(selector);
    return el ? el.textContent.trim() : '';
  }
  for (const sel of ['article', 'main', '[role="main"]', '.content', '#content']) {
    const el = doc.querySelector(sel);
    if (el && el.textContent.trim().length > 100) return el.textContent.trim();
  }
  const body = doc.querySelector('body');
  if (!body) return doc.documentElement.textContent.trim();
  for (const tag of ['script','style','nav','header','footer','aside']) {
    body.querySelectorAll(tag).forEach(el => el.remove());
  }
  dom.window.close();
  return body.textContent.trim().replace(/\n{3,}/g, '\n\n');
}

(async () => {
  try {
    const html = await fetchHTML(url);
    const start = Date.now();
    let text;
    if (engine === 'jsdom') {
      text = await extractJSDOM(html, url);
    } else {
      text = await extractLinkedom(html);
    }
    const elapsed = Date.now() - start;
    // Detect if content looks empty (JS-rendered SPA)
    const jsShell = text.length < 50 && (html.includes('id="root"') || html.includes('id="app"') || html.includes('id="__next"'));
    console.log(JSON.stringify({
      text: text.slice(0, 50000),
      length: text.length,
      truncated: text.length > 50000,
      engine: engine,
      elapsed_ms: elapsed,
      js_shell_detected: jsShell,
      html_size: html.length
    }));
  } catch (e) {
    console.log(JSON.stringify({error: e.message, engine: engine}));
    process.exit(1);
  }
})();
