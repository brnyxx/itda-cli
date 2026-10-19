# 아키텍처

이 문서는 저장소 구조와 Hycli에서 무엇을 가져오고 무엇을 바꿨는지 기록한다.
배경은 프로젝트 인수인계 문서와 `plans/hycli-cycle-breakup.md`(프로젝트 공유 폴더)에 있다.

## 계층

크레이트는 자기보다 아래 계층만 의존한다. Cargo가 순환을 막고, `cargo xtask check-layers`가 방향을 검사한다.

```
0 types   에러 계약, URL 검사, 리댁션. 다른 워크스페이스 크레이트에 의존하지 않음
1 html    스크립트 실행 없는 HTML·SSR JSON 추출
2 spec    SiteSpec 형식, 요청 조립, 응답 검증 (ResponseLike 트레이트로 net과 분리)
3 policy  읽기/쓰기 분류, spec 안전성 검증, 1회용 승인
4 net     HTTP 실행, 속도 조절(Gate), 차단 분류, 사설 주소 차단
5 vault   (예정) 계정·자격증명 저장
6 browser (예정) 세션 가져오기, 렌더링 읽기
7 runtime (예정) 위 크레이트를 조립해 액션 실행
8 prep    (예정) LLM 기반 사이트 분석. runtime은 트레이트로만 사용
9 mcp, server (예정)
10 apps/cli, apps/desktop (예정)
```

인수인계 문서의 목표 구조에서 바뀐 점:
- `types`를 맨 아래에 추가했다. 에러, URL, 리댁션은 모든 층이 쓰므로 spec 아래에 있어야 한다.
- `html`을 spec 아래에 추가했다. spec의 응답 기대값(`required_html_fields`, `html`)이 HTML 추출 타입을 담기 때문이다. spec 자체는 여전히 I/O가 없다.
- 응답 검증(`matches_response`)은 spec에 남기되 `ResponseLike` 트레이트를 받는다. 그래서 spec은 net을 모른다.

## Hycli에서 가져온 것

| 이 저장소 | Hycli 원본 | 바꾼 점 |
|---|---|---|
| `types/error.rs` | `src/apperr.rs` | 안내 문구에서 제품명 제거, rusqlite 변환은 `sqlite` feature로 |
| `types/util.rs` | `src/util.rs` | 임시 파일 접두사만 변경 |
| `types/url.rs` | `src/net.rs`, `src/runtime.rs`의 URL 헬퍼 | 등록 도메인 판정을 손으로 쓴 22개 목록 대신 Public Suffix List(`psl`)로. `a.github.io`와 `b.github.io`를 같은 사이트로 보던 문제 해결 |
| `types/json.rs` | `src/html.rs`의 `resolve_pointer` | 위치만 이동 |
| `html` | `src/html.rs` 일부 | 실행·검증에 필요한 부분만. 특정 중고거래 사이트용 Remix 규칙 삭제. 준비용 증거 수집 함수는 prep으로 갈 예정 |
| `spec` | `src/spec.rs` | `Effect`를 policy에서 이동, `ResponseLike` 도입. 형식 자체는 동일 |
| `policy/classify.rs` | `src/policy.rs` 분류 함수 | 로직 동일 |
| `policy/approvals.rs` | `src/policy.rs` 승인 부분 | 속도 조절과 분리 |
| `policy/validate.rs` | `src/policy.rs` `validate_spec`, `src/session_recipe.rs` `allowed_header` | 의존 정리만 |
| `net/gate.rs` | `src/policy.rs` 속도 조절 부분 | 차단·세션 만료 시 일시정지 추가, 브라우저 전환용 전략 테이블 삭제 |
| `net/defend.rs` | `src/defend.rs` | 안내 문구에서 "브라우저로 전환하라"는 내용 삭제 |
| `net/lib.rs` | `src/net.rs` | 아래 "원칙 변경" 참고 |

## 원칙 변경 (Hycli 대비)

1. **브라우저 흉내를 내지 않는다.** Hycli는 Chrome User-Agent와 `sec-ch-ua`, `sec-fetch-*` 헤더를 보냈다. 이 저장소는 `itda/<버전>`으로 자신을 밝히고 일반적인 Accept 헤더만 보낸다. 사이트가 다른 UA를 요구하면 spec의 `defaults.headers`에 명시해야 하고, 그 선택은 spec 리뷰에서 보인다.
2. **차단을 우회하지 않는다.** Hycli는 직접 요청이 Cloudflare·봇 차단·CAPTCHA를 만나면 브라우저 안에서 같은 요청을 다시 보내는 경로(`fetch_browser_in_page`, `fetch_rendered`)로 넘어갔다. 이 저장소에는 그 경로가 없다. 차단을 분류해 보고하고, 그 사이트(모든 계정)를 사용자가 재개할 때까지 멈춘다.
3. **로그아웃과 차단을 구분한다.** 로그인한 계정의 요청이 401을 받으면 그 계정만 `auth_required`로 멈춘다. 같은 사이트의 익명 읽기는 영향받지 않는다.

spec의 `transport: browser` / `browser-fetch` 필드는 형식 호환을 위해 남아 있다. 실행은 browser 크레이트가 생길 때 "spec이 명시한 경우에만, 사용자 세션으로" 지원할지 결정한다.

## 다음 단계

1. `vault`: 계정·자격증명(OS 키링, 암호화 파일). Hycli `accounts.rs`, `store.rs`, `session_recipe.rs` 데이터 부분 참고
2. `runtime`: spec 실행, 승인 흐름, 결과 리댁션. Hycli `runtime.rs` 실행 경로만(정찰·AI 설정 제외)
3. `mcp` + `apps/cli`: rmcp 기반 MCP 서버와 CLI. Hycli `guarded_mcp.rs` 참고
4. 배포: cargo-dist로 3개 OS 설치
