//! Browser fingerprint management — anti-detection measures for headless/automated Chrome.
//! Injected via evaluate() after launch or attach when stealth=true.

/// JavaScript to inject that removes common automation indicators.
/// Should be evaluated on every new page load (or via addScriptToEvaluateOnNewDocument).
pub const STEALTH_JS: &str = r#"
(() => {
    // 1. Remove WebDriver indicator
    Object.defineProperty(navigator, 'webdriver', {
        get: () => false,
        configurable: true,
    });

    // 2. Remove chrome.cdc_* properties (ChromeDriver markers)
    if (window.chrome) {
        const keysToDelete = Object.keys(window.chrome).filter(k => k.startsWith('cdc_'));
        keysToDelete.forEach(k => { delete window.chrome[k]; });
    }

    // 3. Override navigator.plugins with realistic values
    Object.defineProperty(navigator, 'plugins', {
        get: () => {
            const arr = [
                { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
                { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
                { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' },
            ];
            arr.length = 3;
            arr.item = i => arr[i] || null;
            arr.namedItem = n => arr.find(p => p.name === n) || null;
            arr.refresh = () => {};
            return arr;
        },
        configurable: true,
    });

    // 4. Override navigator.languages
    Object.defineProperty(navigator, 'languages', {
        get: () => ['en-US', 'en'],
        configurable: true,
    });

    // 5. Set proper navigator.platform and vendor
    Object.defineProperty(navigator, 'platform', {
        get: () => 'Win32',
        configurable: true,
    });
    Object.defineProperty(navigator, 'vendor', {
        get: () => 'Google Inc.',
        configurable: true,
    });

    // 6. WebGL renderer spoofing
    const getParameterOrig = WebGLRenderingContext.prototype.getParameter;
    WebGLRenderingContext.prototype.getParameter = function(param) {
        // UNMASKED_VENDOR_WEBGL
        if (param === 0x9245) return 'Google Inc. (NVIDIA)';
        // UNMASKED_RENDERER_WEBGL
        if (param === 0x9246) return 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1650 Direct3D11 vs_5_0 ps_5_0, D3D11)';
        return getParameterOrig.call(this, param);
    };
    if (typeof WebGL2RenderingContext !== 'undefined') {
        const getParameter2Orig = WebGL2RenderingContext.prototype.getParameter;
        WebGL2RenderingContext.prototype.getParameter = function(param) {
            if (param === 0x9245) return 'Google Inc. (NVIDIA)';
            if (param === 0x9246) return 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1650 Direct3D11 vs_5_0 ps_5_0, D3D11)';
            return getParameter2Orig.call(this, param);
        };
    }

    // 7. Consistent canvas fingerprint
    const toDataURLOrig = HTMLCanvasElement.prototype.toDataURL;
    HTMLCanvasElement.prototype.toDataURL = function(type) {
        const ctx = this.getContext('2d');
        if (ctx) {
            // Add invisible noise to make fingerprint non-default but consistent
            const imgData = ctx.getImageData(0, 0, 1, 1);
            imgData.data[3] = imgData.data[3] === 0 ? 0 : imgData.data[3];
            ctx.putImageData(imgData, 0, 0);
        }
        return toDataURLOrig.apply(this, arguments);
    };
})();
"#;

// Note: --disable-blink-features=AutomationControlled is already applied by
// browser-mcp's launch() function. The JS above handles the remaining indicators.
