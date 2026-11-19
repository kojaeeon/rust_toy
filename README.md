# Rust Toy Projects

작은 Rust 도구를 모아 둔 저장소입니다. 각 폴더는 독립적인 Cargo 프로젝트입니다.

| 프로젝트 | 설명 |
|---|---|
| [hotkey](hotkey/) | Windows 단축키 도구 |
| [logwatch](logwatch/) | Java·SQL·XML 로그를 실시간으로 검색하고 확인하는 터미널 뷰어 |

## Logwatch 실행

```powershell
cd logwatch
cargo run --release
```

설정, 조작법과 실시간 데모는 [Logwatch 안내](logwatch/README.md)를 참고하세요.
