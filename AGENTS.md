# AGENTS.md — Regras de Trabalho do Projeto KnobContext

## Regra #1: Contexto Primeiro
- **Antes de iniciar qualquer trabalho**, leia `CONTEXT.md` na raiz do projeto para conhecer o estado atual, decisões tomadas, limitações conhecidas e próximos passos.
- Ao concluir uma etapa ou descoberta relevante (especialmente sobre hardware/HID), atualize `CONTEXT.md`.

## Padrões de Código Rust
- Edição Rust 2021+.
- Todo código deve passar em `cargo fmt --check` e `cargo clippy -- -D warnings`.
- Trate erros de forma explícita (sem `.unwrap()` ou `.expect()` em código de produção, exceto quando a invariante for documentada).
- Mantenha consumo de memória baixo (meta < 50MB).

## Testes e Qualidade
- Novas funcionalidades e parsers devem vir acompanhadas de testes unitários (`cargo test`).
- Não adicione dependências não utilizadas ao `Cargo.toml`.

## Hardware e Protocolo HID
- **Não assuma o comportamento do hardware**: execute probes e obtenha dados reais antes de projetar parsers ou protocolos de escrita.
- Em caso de dúvida sobre o protocolo HID do dispositivo, pare e pergunte ao usuário em vez de adivinhar.
- No Linux, prefira caminhos persistentes em `/dev/input/by-id/` em vez de `/dev/input/eventN`.

## Arquitetura e Controle de Versão
- **Alterações arquiteturais importantes** (mudança de crates principais, protocolo IPC, estrutura de módulos ou fluxo de dados) devem ser propostas primeiro e aguardar confirmação do usuário antes da implementação.
- Faça **commits pequenos e frequentes**: uma funcionalidade verificável por commit.
