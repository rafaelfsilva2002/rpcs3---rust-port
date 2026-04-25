# Contracts — testes GTest plugáveis

Estes arquivos **reaproveitam o harness existente em `rpcs3/tests/`**. Eles não refatoram nada — apenas adicionam novos `TEST(...)` que checam contratos estáveis.

## Como integrar

Na build existente, edite `rpcs3/CMakeLists.txt` entre as linhas 182–194 (`target_sources(rpcs3_test PRIVATE ...)`) e adicione:

```cmake
target_sources(rpcs3_test
    PRIVATE
        tests/test.cpp
        tests/test_fmt.cpp
        tests/test_pair.cpp
        tests/test_tuple.cpp
        tests/test_simple_array.cpp
        tests/test_address_range.cpp
        tests/test_sys_fs.cpp
        tests/test_rsx_cfg.cpp
        tests/test_rsx_fp_asm.cpp
        tests/test_dmux_pamf.cpp
        # Behavior-freeze contracts:
        ${CMAKE_SOURCE_DIR}/behavior-freeze/contracts/test_contract_game_boot_result.cpp
        ${CMAKE_SOURCE_DIR}/behavior-freeze/contracts/test_contract_system_state.cpp
        ${CMAKE_SOURCE_DIR}/behavior-freeze/contracts/test_contract_cpu_flag.cpp
)
```

Depois:

```bash
cmake -S . -B build -DBUILD_RPCS3_TESTS=ON
cmake --build build --target rpcs3_test
ctest --test-dir build --output-on-failure -R Contract
```

## Por que essa integração não é automática

A decisão de modificar o `rpcs3/CMakeLists.txt` fica com o humano. São 3 linhas de inclusão — a mudança é mínima, mas toca código de produção e portanto exige confirmação explícita.
