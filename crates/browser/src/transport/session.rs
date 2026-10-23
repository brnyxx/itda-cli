use super::{BODY_LIMIT, Browser, Call, Page, credentials, protocol::field};
use itda_types::error::AppResult;
use serde_json::json;
use std::time::{Duration, Instant};

impl Browser {
    pub(super) fn prepare(&mut self, call: &mut Call<'_>) -> AppResult<()> {
        let fingerprint = credentials::fingerprint(call.request, call.url)?;
        if self.credential_digest.as_ref() != Some(&fingerprint) {
            if let Some(context) = self.context.take() {
                self.command(
                    None,
                    "Target.disposeBrowserContext",
                    json!({"browserContextId": context}),
                    call,
                )?;
            }
            self.page = None;
            self.credential_cookies.clear();
            self.credential_digest = Some(fingerprint);
        }
        let context = match &self.context {
            Some(context) => context.clone(),
            None => {
                let context = self.command(
                    None,
                    "Target.createBrowserContext",
                    json!({
                        "disposeOnDetach":true,
                        "proxyServer":format!("socks5://{}", self.proxy.address()),
                        "proxyBypassList":"<-loopback>",
                    }),
                    call,
                )?;
                let context = field(&context, "browserContextId")?;
                self.context = Some(context.clone());
                context
            }
        };
        if self.page.is_none() {
            self.command(
                None,
                "Fetch.enable",
                json!({
                    "patterns":[
                        {"urlPattern":"*","requestStage":"Request"},
                        {"urlPattern":"*","requestStage":"Response"},
                    ],
                    "handleAuthRequests":true,
                }),
                call,
            )?;
            self.command(
                None,
                "Target.setAutoAttach",
                json!({
                    "autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,
                    "filter":[{"type":"shared_worker"},{"type":"service_worker"},{"exclude":true}],
                }),
                call,
            )?;
            let target = self.command(
                None,
                "Target.createTarget",
                json!({"url":"about:blank", "browserContextId":context}),
                call,
            )?;
            let target = field(&target, "targetId")?;
            let attached = self.command(
                None,
                "Target.attachToTarget",
                json!({"targetId":target, "flatten":true}),
                call,
            )?;
            let session = field(&attached, "sessionId")?;
            call.session = session.clone();
            call.sessions.insert(session.clone());
            self.command(Some(&session), "Page.enable", json!({}), call)?;
            self.command(Some(&session), "ServiceWorker.enable", json!({}), call)?;
            let tree = self.command(Some(&session), "Page.getFrameTree", json!({}), call)?;
            let frame = field(&tree["frameTree"]["frame"], "id")?;
            call.frame = frame.clone();
            self.configure(&session, true, call)?;
            self.page = Some(Page { session, frame });
        }
        if let Some(page) = &self.page {
            call.session = page.session.clone();
            call.frame = page.frame.clone();
            call.sessions.insert(page.session.clone());
        }
        let session = call.session.clone();
        self.command(
            Some(&session),
            "Emulation.setScriptExecutionDisabled",
            json!({"value":false}),
            call,
        )?;
        let cookies = credentials::cookies(call.request)?;
        for name in self
            .credential_cookies
            .keys()
            .filter(|name| !cookies.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>()
        {
            self.command(
                Some(&session),
                "Network.deleteCookies",
                json!({"name":name, "url":self.origin}),
                call,
            )?;
        }
        let delta = cookies
            .iter()
            .filter(|(name, value)| self.credential_cookies.get(*name) != Some(*value))
            .map(|(name, value)| {
                json!({
                    "name":name,"value":value,"url":self.origin,"path":"/",
                    "sourcePort":call.url.port_or_known_default(),
                })
            })
            .collect::<Vec<_>>();
        if !delta.is_empty() {
            self.command(
                Some(&session),
                "Network.setCookies",
                json!({"cookies":delta}),
                call,
            )?;
        }
        self.credential_cookies = cookies;
        let agent = call
            .request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        self.command(
            Some(&session),
            "Network.setUserAgentOverride",
            json!({"userAgent":agent}),
            call,
        )?;
        Ok(())
    }

    pub(super) fn configure(
        &mut self,
        session: &str,
        page: bool,
        call: &mut Call<'_>,
    ) -> AppResult<()> {
        self.command(
            Some(session),
            "Network.enable",
            json!({
                "maxTotalBufferSize": call.request.limit.min(BODY_LIMIT),
                "maxResourceBufferSize": call.request.limit.min(BODY_LIMIT),
            }),
            call,
        )?;
        self.command(
            Some(session),
            "Network.setBypassServiceWorker",
            json!({"bypass":true}),
            call,
        )?;
        self.command(
            Some(session),
            "Network.setCacheDisabled",
            json!({"cacheDisabled":true}),
            call,
        )?;
        self.command(
            Some(session),
            "Network.setBlockedURLs",
            json!({"urls":["ws://*", "wss://*"]}),
            call,
        )?;
        self.command(
            Some(session),
            "Target.setAutoAttach",
            json!({
                "autoAttach":true, "waitForDebuggerOnStart":true, "flatten":true,
                "filter":[{"type":"shared_worker","exclude":true},{"type":"service_worker","exclude":true},{}],
            }),
            call,
        )?;
        if page {
            self.command(Some(session), "Page.addScriptToEvaluateOnNewDocument", json!({
                "source":"Object.defineProperty(Navigator.prototype,'webdriver',{get:()=>false}); Object.defineProperty(window,'open',{value:()=>null}); Object.defineProperty(window,'RTCPeerConnection',{value:undefined}); Object.defineProperty(window,'webkitRTCPeerConnection',{value:undefined});",
            }), call)?;
        }
        Ok(())
    }

    pub(super) fn settle(&mut self, call: &mut Call<'_>) -> AppResult<()> {
        self.proxy.deactivate()?;
        call.settling = true;
        call.deadline = Instant::now() + Duration::from_secs(2);
        let session = call.session.clone();
        let world = self.command(
            Some(&session),
            "Page.createIsolatedWorld",
            json!({"frameId":call.frame,"worldName":"itda-cleanup"}),
            call,
        )?;
        self.command(
            Some(&session),
            "Runtime.evaluate",
            json!({
                "expression":"document.open();document.close()",
                "contextId":world["executionContextId"],
                "returnByValue":true,
            }),
            call,
        )?;
        self.command(
            Some(&session),
            "Emulation.setScriptExecutionDisabled",
            json!({"value":true}),
            call,
        )?;
        self.command(Some(&session), "Page.stopLoading", json!({}), call)?;
        self.command(
            Some(&session),
            "ServiceWorker.stopAllWorkers",
            json!({}),
            call,
        )?;
        let shared = self.command(
            None,
            "Target.getTargets",
            json!({"filter":[{"type":"shared_worker"}]}),
            call,
        )?;
        for target in shared["targetInfos"].as_array().into_iter().flatten() {
            if target["browserContextId"].as_str() == self.context.as_deref() {
                self.command(
                    None,
                    "Target.closeTarget",
                    json!({"targetId":target["targetId"]}),
                    call,
                )?;
            }
        }
        call.loaded = false;
        self.command(
            Some(&session),
            "Page.navigate",
            json!({"url":"about:blank"}),
            call,
        )?;
        while !call.loaded {
            let event = self.read(call.deadline)?;
            self.event(&event, call)?;
        }
        Ok(())
    }
}
