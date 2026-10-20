<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/wordmark-dark.svg">
    <img src="assets/wordmark-light.svg" alt="itda" width="240">
  </picture>
</p>

<p align="center"><strong>브라우저 조종 말고, 사이트를 컴파일하세요.</strong></p>

<p align="center"><a href="README.md">English</a> · <b>한국어</b></p>

<p align="center">
  <a href="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml"><img src="https://github.com/brnyxx/itda-cli/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/brnyxx/itda-cli/releases"><img src="https://img.shields.io/github/v/release/brnyxx/itda-cli?include_prereleases&amp;label=release" alt="Release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License: Apache-2.0"></a>
  <a href="https://brnyxx.github.io/itda-cli/?lang=ko">홈페이지</a> ·
  <a href="https://brnyxx.github.io/itda-cli/docs/">문서(영어)</a>
</p>

itda는 내가 쓰는 웹사이트를, 로그인이 필요한 오래된 사내 시스템까지도, AI
에이전트가 명령줄이나 MCP로 호출할 수 있는 검증된 명령 묶음으로 바꿉니다.
브라우저에서 사이트를 쓰는 모습을 한 번 녹화하면 itda가 그 녹화로 명령을
만듭니다. 내 컴퓨터에서 실행되고, 기본은 읽기만 하며, 바로 그 변경을 내가
승인하기 전에는 사이트에서 아무것도 바꾸지 않습니다.

<p align="center"><img src="assets/flow.ko.svg" alt="브라우저에서 웹사이트를 한 번 녹화하면 itda가 명령을 만들고, 에이전트가 호출하며, 변경은 사람이 승인할 때까지 기다립니다." width="860"></p>

> 상태: 초기 단계이며 실제 사이트에서 쓰고 있습니다. 런타임, CLI, MCP 서버,
> 승인(터미널 또는 에이전트 앱 안에서), 감사 로그, OpenAPI 문서나 브라우저
> 녹화로 초안 만들기, API 키, 브라우저 로그인, CSRF 사전 단계가 지금 동작합니다.
> 서명된 사이트 팩이 다음 순서입니다. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)를
> 보세요.

## 왜 필요한가

브라우저를 조종하는 에이전트는 매번 페이지를 다시 읽고, 스크린샷이나 접근성
스냅샷마다 수천 토큰을 쓰고, 페이지가 시키는 대로 클릭하고, 화면 배치가 바뀌면
조용히 망가집니다. itda는 그 일을 설치 단계로 옮깁니다.

| | 브라우저 조종 에이전트 | itda |
|---|---|---|
| 에이전트가 보는 것 | 매 단계 페이지 전체 | 액션 하나의 정해진 결과, 최대 24 KB |
| 바뀌었거나 로그아웃된 페이지 | 그대로 데이터로 읽음 | 선언된 응답과 비교해 감지 (`response_mismatch`, `auth_required`) |
| 사이트를 바꾸는 일 | 모델이 정하는 대로 | 정확한 입력값에 대한 1회용 승인, 터미널이나 에이전트 앱에서 사람이 직접 |
| 페이지 속 "이전 지시는 무시해" | 프롬프트에 그대로 들어감 | 신뢰할 수 없는 데이터로 표시, 도구가 되지 않음 |
| 인증 정보 | 모델이 페이지에 직접 입력 | OS 키링에 보관, 에이전트는 계정 이름만 봄 |
| 차단당했을 때 | 재시도, IP 바꾸기, 브라우저 흉내 | 멈추고, 알리고, 내가 재개할 때까지 기다림 |
| 무슨 일이 있었는지 | 대화 기록 | 검증할 수 있는 해시 체인 감사 로그 |

## 빠른 시작

```sh
# macOS, Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.sh | sh
```

```powershell
# Windows
powershell -ExecutionPolicy Bypass -c "irm https://github.com/brnyxx/itda-cli/releases/latest/download/itda-cli-installer.ps1 | iex"
```

[Releases](https://github.com/brnyxx/itda-cli/releases)에서 압축 파일을 받거나,
`cargo install --git https://github.com/brnyxx/itda-cli itda-cli`로 소스에서
빌드해도 됩니다.

그다음 내가 쓰는 웹사이트를 명령으로 바꿔 보세요. 브라우저 개발자 도구의
Network 탭을 열고 평소처럼 사이트를 쓴 뒤(로그인, 검색, 폼 전송) 기록을 HAR
파일로 저장합니다.

```sh
itda draft har board.har --name board --install   # 찾은 내용을 보여줌, 값은 절대 복사 안 함
itda login board --browser                          # 내 브라우저로 한 번 로그인
itda call board.list-notices --json                 # 읽기는 바로 실행
itda call board.create-notice --title "서버 점검 안내"   # 변경은 내 승인을 기다림
itda mcp install claude-desktop                     # 에이전트가 호출하게 연결
```

공개 API는 녹화가 필요 없습니다. [sites/](sites/README.md)의 기본 사이트는
itda에 들어 있고 계정도 필요 없습니다.

<p><a href="sites/github.yaml"><img src="assets/brands/github.svg" width="36" height="36" alt="GitHub"></a> <a href="sites/npm.yaml"><img src="assets/brands/npm.svg" width="36" height="36" alt="npm"></a> <a href="sites/crates-io.yaml"><img src="assets/brands/crates-io.svg" width="36" height="36" alt="crates.io"></a> <a href="sites/pypi.yaml"><img src="assets/brands/pypi.svg" width="36" height="36" alt="PyPI"></a></p>

```sh
itda add github      # npm, crates-io, pypi도 같은 방식
itda call github.repo --owner rust-lang --repo rust
```

127.0.0.1에서만 열리는 가짜 사내 시스템인 샌드박스로도 해 볼 수 있습니다.
[examples/sandbox](examples/sandbox/README.md)를 따라 하세요. 사이트 정의는 YAML
파일 하나입니다. `itda add spec.yaml`은 요청을 어디로 보내는지와 어떤 액션이
사이트를 바꾸는지 보여준 뒤 설치하고, `itda doctor`는 데이터 폴더 위치를
알려줍니다. 정의를 직접 쓰는 방법은 [docs/SPEC.md](docs/SPEC.md)에 있습니다.

```sh
itda add spec.yaml / remove <site>       # 사이트 설치·삭제 (URL이나 github 같은 이름도 가능)
itda ls                                  # 설치된 사이트와 액션
itda call <site>.<action> --input value  # 하나 실행, 에이전트용은 --json
echo "$KEY" | itda login <site>          # API 키를 OS 키링에 저장
itda login <site> --browser              # 또는 내 브라우저로 한 번 로그인
itda accounts / logout <site>            # 저장된 로그인 보기·삭제
itda approvals / approve <id> / reject <id>
itda audit [--verify]                    # 에이전트가 한 일
itda check [<site>]                      # 모든 읽기가 아직 사이트와 맞는지 확인
itda resume <site>                       # 사이트가 거부했거나 키가 만료된 뒤 재개
```

## 에이전트에서 쓰기

```sh
itda mcp install claude-desktop   # 또는 cursor, vscode, codex
itda mcp install claude-code      # `claude mcp add` 명령을 출력
# --approve-in-client를 붙이면 클라이언트 자체 양식에서 변경을 승인
```

설치 명령은 클라이언트 설정에 병합되고, 다른 서버 설정은 그대로 두며, 기존
파일은 `.bak`으로 백업합니다. `--dry-run`으로 바뀔 내용을 먼저 볼 수 있습니다.
`itda mcp`는 설치된 모든 사이트를 제공합니다.

- 액션이 30개까지는 각각 도구 하나로 보입니다(`sandbox.list-notices`). 입력
  스키마와 읽기 전용·파괴적·멱등 힌트가 붙습니다.
- 그보다 많으면 고정된 도구 세 개(`itda_search`, `itda_describe`, `itda_call`)로
  도구 목록을 짧게 유지해 프롬프트 캐시가 깨지지 않습니다. `--mode`로 바꿀 수
  있습니다.
- 결과는 `itda call --json`과 같은 JSON 형식이고, 신뢰할 수 없음으로 표시됩니다.
- 변경은 승인 번호와 함께 `approval_required`를 돌려줍니다. 승인하는 도구는
  없으므로 에이전트는 사람에게 부탁해야 합니다.

셸만 쓸 수 있는 에이전트는 `itda call … --json`을 호출하고 종료 코드로 분기하면
됩니다.

| 종료 코드 | 뜻 |
|---|---|
| 0 | 성공 |
| 2 | 잘못된 입력이거나 없는 액션 |
| 10 | 다시 로그인 필요 (`itda login`) |
| 11 | 페이지가 더 이상 정의와 맞지 않음 |
| 12 | 사이트가 거부함, `itda resume` 전까지 일시 정지 |
| 13 | 요청 한도 초과 |
| 20 | 승인 필요 (또는 변경이 꺼져 있음) |
| 21 | 승인이 이미 쓰였거나, 거절·만료됐거나, 다른 입력용임 |
| 22 | 변경을 보냈지만 결과를 알 수 없음, 사이트를 확인하세요 |
| 30 | 네트워크 오류 |

## 원칙

- **로컬 우선.** 정의는 내 기기에서 실행됩니다. 인증 정보는 기기를 떠나지
  않습니다.
- **기본은 읽기 전용.** 메서드, 이름, 경로, 입력, GraphQL 문서가 모두 읽기라고
  말할 때만 읽기입니다. 나머지는 모두 변경입니다.
- **변경은 사람이 승인.** 승인 하나는 정확한 입력값, 계정, 정의, 인증 정보를
  가진 요청 하나에만 쓰이고, 한 번 쓰면 끝나며, 스스로 만료됩니다.
- **차단은 우회하지 않고 알립니다.** itda는 자신이 누구인지 밝히고, 브라우저를
  흉내 내지 않으며, 거부하는 사이트에서는 멈춥니다.
  [docs/HONEST_CLIENT.md](docs/HONEST_CLIENT.md)를 보세요.
- **검증된 액션.** 정의는 올바른 응답의 모양을 선언하므로, "로그아웃됨"과
  "사이트가 바뀜"은 데이터가 아니라 오류입니다.

## 구조

크레이트는 자기보다 아래 크레이트에만 의존하고, `cargo xtask check-layers`가
이를 검사합니다.

| 계층 | 크레이트 | 역할 |
|---|---|---|
| 0 | `itda-types` | 오류 형식, URL 검사(Public Suffix List), 민감 정보 가리기 |
| 1 | `itda-html`, `itda-store` | 스크립트 없는 HTML 추출, 로컬 SQLite 저장소 |
| 2 | `itda-spec` | 정의 형식, 요청 만들기, 응답 검증 |
| 3 | `itda-policy` | 읽기/변경 분류, 정의 안전성 검사, 승인 |
| 4 | `itda-net` | 제한된 HTTP, 속도 조절, 차단 감지, 사설 주소 차단 |
| 5 | `itda-vault` | OS 키링이나 환경 변수 속 계정과 비밀 정보 |
| 7 | `itda-runtime` | 액션 실행: 인증 정보, 승인, 결과 정리, 감사 로그 |
| 9 | `itda-mcp` | stdio MCP 서버 |
| 10 | `apps/cli` | `itda` 명령 |

## 개발

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check-layers
```

Rust 1.98(edition 2024)이 `rust-toolchain.toml`에 고정되어 있습니다. 테스트는
스스로 띄운 로컬 서버하고만 통신합니다. 커밋과 PR 규칙은
[CONTRIBUTING.md](CONTRIBUTING.md)에 있습니다.

## 보안

[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md)에 itda가 막는 것과 막지 않는 것이
정리되어 있습니다. 취약점은 [SECURITY.md](SECURITY.md)에 적힌 방법으로 알려
주세요.

## 출처와 라이선스

[Apache-2.0](LICENSE) 라이선스입니다. 이 프로젝트의 일부는
[Hycli](https://github.com/Hybirdss/Hycli)(Apache-2.0)에서 가져왔습니다.
[NOTICE](NOTICE)를 보세요. 가져온 파일마다 머리말에 그 사실과 바뀐 점을 적어
두었습니다. 이 프로젝트는 Hycli와 관련이 없으며 그 이름이나 표장을 쓰지
않습니다.

이 문서는 [README.md](README.md)의 번역입니다. 내용이 다르면 영어판이
기준입니다.
