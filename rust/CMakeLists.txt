# Integração CMake ↔ Cargo via corrosion.
#
# Dois modos de uso:
#
#  1. Via `add_subdirectory(rust)` do CMakeLists.txt raiz (quando
#     -DUSE_RUST_CRATES=ON). Herda o project() pai.
#  2. Standalone, para smoke-test da integração sem puxar 3rdparty/:
#        cmake -S rust -B build-rust-smoke
#        cmake --build build-rust-smoke --target rpcs3_utilities_rs
#
# Não modifica nenhum target existente do rpcs3 — apenas cria novos
# targets (rpcs3_utilities_rs, etc.) que podem ser linkados no futuro.

cmake_minimum_required(VERSION 3.28)

# project() só se esta for a raiz do build (modo standalone).
if(CMAKE_SOURCE_DIR STREQUAL CMAKE_CURRENT_SOURCE_DIR)
    project(rpcs3_rust LANGUAGES C CXX)
endif()

include(FetchContent)

FetchContent_Declare(
    Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG v0.5.1
)
FetchContent_MakeAvailable(Corrosion)

corrosion_import_crate(
    MANIFEST_PATH "${CMAKE_CURRENT_SOURCE_DIR}/Cargo.toml"
    PROFILE "$<IF:$<CONFIG:Debug>,dev,release>"
    CRATE_TYPES staticlib
)

# Corrosion creates CMake targets named after the Cargo [lib] name
# (underscores, Rust identifier convention), NOT the [package] name.
# Our crates expose: rpcs3_utilities, rpcs3_config, rpcs3_crypto.
# Expose include directories for the ones that publish C headers.
target_include_directories(rpcs3_utilities INTERFACE
    "${CMAKE_CURRENT_SOURCE_DIR}/rpcs3-utilities/include"
)

message(STATUS "Rust crates enabled. Workspace: ${CMAKE_CURRENT_SOURCE_DIR}")
