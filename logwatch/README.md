# Logwatch

ezTerm의 PowerShell/CMD에서 실행하는 Rust 실시간 로그 뷰어입니다. Java 로그, SQL, XML, 예외 스택을 시간순으로 읽고 선택한 이벤트를 상세 화면에서 확인합니다.

## 실행

Rust 도구 체인이 설치된 환경에서 프로젝트 폴더를 열어 실행하세요.

```powershell
cd C:\kkk\Personal_Projects\rust_toy\logwatch
cargo run --release
```

다른 로그 파일을 열 때:

```powershell
cargo run --release -- C:\server\logs\application.log
```

빌드 후에는 Rust 없이 `target\release\logwatch.exe`를 직접 실행할 수 있습니다. 설정을 함께 사용하려면 TOML 경로를 지정하세요.

```powershell
.\target\release\logwatch.exe --config .\logwatch.toml
.\target\release\logwatch.exe C:\server\logs\application.log --config .\logwatch.toml
.\target\release\logwatch.exe --dump
```

`--dump`는 최근 로그를 한 번 해석해 출력하고 종료합니다. ezTerm 자체와의 통합 플러그인이 아닌 터미널 프로그램이며, SSH로 연결한 서버에서 실행하려면 해당 서버 OS용으로 빌드해야 합니다.

## 실시간 데모

ezTerm 탭 하나에서 예제 로그를 조금씩 기록합니다. `live-demo.log`는 데모를 시작할 때 초기화됩니다.

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\replay.ps1 -Repeat
```

다른 탭에서 다음을 실행하세요.

```powershell
.\target\release\logwatch.exe .\logs\live-demo.log
```

`-DelayMs 1500`으로 이벤트 간격을 바꿀 수 있습니다. 반복 재생은 예제의 타임스탬프를 그대로 사용합니다.

## 화면과 조작

넓은 터미널에서는 좌측 40%에 로그 목록, 우측 60%에 상세 내용을 표시합니다. 95열 미만에서는 상하 배치로 자동 전환합니다. 최소 크기는 50열 × 12행입니다.

| 키 | 동작 |
|---|---|
| ↑↓ / j k | 이벤트 선택, 자동 추적 해제 |
| Home / End | 첫 이벤트 / 최신 이벤트와 자동 추적 재개 |
| Space | 최신 이벤트 자동 추적 켜기/끄기 |
| / | 원문 전체에서 대소문자 구분 없이 검색 |
| Enter / Esc | 검색 입력 완료; 입력 중 Ctrl+U로 검색어 삭제 |
| Esc (검색 입력 밖) | 검색어·요청 ID 필터 해제 |
| f | 전체 → INFO 이상 → WARN 이상 → ERROR |
| t | 전체 → JAVA → SQL → XML |
| r | 선택 이벤트의 requestId/traceId로 필터; 다시 누르면 해제 |
| Tab | 목록/상세에 키보드 초점 이동 |
| 상세 영역의 ↑↓ / PgUp PgDn / Home End | 본문 스크롤 / 처음·끝으로 이동 |
| 상세 영역의 ←→ | 줄바꿈을 껐을 때 가로 스크롤 |
| v | 포맷 보기/원문 전환 |
| w | 상세 본문 줄바꿈 전환 |
| l | 좌우/상하 배치 전환 |
| ? / F1 | 도움말 |
| q / Ctrl+C | 종료 |

상세 본문이 화면보다 길면 제목에 현재 표시 줄과 전체 줄 수가 나타납니다. `Tab`으로 상세창에 초점을 옮긴 뒤 `End`를 누르면 쿼리 끝으로 이동합니다.

자동 추적을 꺼도 새 로그를 계속 읽습니다. 읽던 이벤트를 고정해서 확인할 수 있고, 메모리 제한으로 해당 이벤트가 제거되면 남아 있는 첫 이벤트를 선택합니다.

## TOML 설정

[logwatch.toml](logwatch.toml)에 화면·보관 개수·폴링 간격·파일 경로·인코딩·여러 줄 묶기 규칙이 있습니다. `--config`가 없으면 현재 폴더의 `logwatch.toml`을 사용하며, 없으면 기본 설정을 사용합니다.

- 설정에 적힌 파일 경로는 **TOML 파일 위치 기준**, 명령행 파일 경로는 **현재 작업 폴더 기준**입니다.
- `[[sources]]`를 여러 개 정의하고 `--source 이름`으로 선택할 수 있습니다. 한 세션에서 파일 하나를 엽니다.
- UTF-8과 EUC-KR/CP949(`encoding = "euc-kr"`) 등을 지원합니다. UTF-16은 지원하지 않습니다.
- `start_pattern`에 맞는 줄부터 다음 시작 줄 전까지 한 이벤트입니다. 기본 Java 헤더 파서는 예제의 `시간 레벨 [스레드] 로거 - 메시지` 형식입니다. 다른 헤더 형식도 원문을 보관하지만 필드 추출은 제한됩니다.
- SQL 바인드·실행시간은 원문에 있는 값만 표시합니다. SQL 키워드·XML 줄·스택에 색상을 적용하고 XML이 잘렸으면 경고와 원문을 표시합니다. XML 노드 접기는 아직 지원하지 않습니다.
- 원문 보기는 디코딩된 텍스트입니다. 터미널 제어 문자는 화면 출력에서 제거하며, 원본 파일은 수정하지 않습니다.

## 읽기 범위와 제한

시작할 때 정확한 파일 줄 번호를 얻기 위해 파일 전체를 순차 스캔하고 마지막 `initial_lines`줄(기본 200)만 보관합니다. 큰 파일은 첫 화면까지 시간이 걸릴 수 있습니다. 시작 범위가 여러 줄 이벤트 중간이면 첫 이벤트는 헤더 없는 원문으로 보입니다.

이후 기본 200ms 간격으로 최대 256KiB씩 추가 바이트를 읽습니다. 라이브 화면에는 개행까지 도착한 줄만 표시하며, `--dump`에서는 마지막 개행 없는 줄도 표시합니다. 파일이 잠시 없어지거나 열리지 않으면 상태 줄에 오류를 표시하고 재시도합니다.

파일 식별 정보·크기·읽은 지점의 마지막 64바이트를 비교해 교체/잘림을 감지합니다. 기존 이벤트는 남겨 두고 세대(`gen`)와 줄 번호를 새로 시작합니다. 폴링 사이에 잘렸다가 같은 식별 정보·동일한 64바이트 내용까지 복구되는 경우는 감지하지 못할 수 있습니다.

기본 보관 한도는 50,000개 이벤트입니다. 한 줄은 64KiB, 한 이벤트 본문은 256KiB로 제한하며 초과 시 상태 줄에 잘림 횟수를 표시합니다. 이 경우 원문 전체는 원본 로그 파일에서 확인해야 합니다. 요청 ID가 없는 여러 스레드의 본문이 줄 단위로 뒤섞이면 자동 복원하지 않습니다.

## 검증

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

예제 이벤트 묶기, SQL/XML/스택, UTF-8 분할 추기, CP949, 교체·잘림·재시도, 줄 번호, 보관 한도, 검색·필터·추적, 넓은/좁은 화면 렌더링을 테스트합니다.
