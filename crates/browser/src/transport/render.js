async (limit, wait) => {
    const challenge = () => {
        const text = `${document.title} ${document.body?.innerText ?? ''}`;
        return /\b(captcha|challenge|checking your browser|verify you are human|just a moment|security check)\b/i.test(text)
            || document.querySelector('iframe[src*="captcha"], [data-sitekey], #challenge-form') !== null;
    };
    if (challenge() && wait > 0) {
        await new Promise(resolve => {
            const signal = AbortSignal.timeout(wait);
            const observer = new MutationObserver(() => {
                if (!challenge()) finish();
            });
            const finish = () => {
                observer.disconnect();
                signal.removeEventListener('abort', finish);
                resolve();
            };
            observer.observe(document, {subtree: true, childList: true, characterData: true, attributes: true});
            signal.addEventListener('abort', finish, {once: true});
            if (!challenge()) finish();
        });
    }
    const bytes = new TextEncoder().encode(document.documentElement.outerHTML);
    if (bytes.length > limit) return {error: 'response_too_large'};
    let binary = '';
    for (let i = 0; i < bytes.length; i += 0x8000) {
        binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }
    return {b64: btoa(binary)};
}
