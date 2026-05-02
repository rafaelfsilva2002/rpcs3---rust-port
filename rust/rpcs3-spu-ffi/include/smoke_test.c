/* R6.0c smoke test — make sure rpcs3_spu_ffi.h is consumable by a
 * plain C compiler. Compile via:
 *   cc -c -Wall -Wextra -Werror -Iinclude include/smoke_test.c -o /tmp/smoke.o
 *
 * This file is NOT linked or executed; it just exercises the header
 * declarations syntactically. It uses every public symbol so the
 * compiler must parse every declaration and resolve every type.
 */

#include "rpcs3_spu_ffi.h"

/* Force the compiler to instantiate every declaration. */
typedef struct {
    rust_spu_t* (*p_new)(void);
    void (*p_drop)(rust_spu_t*);
    int32_t (*p_load_ls)(rust_spu_t*, const uint8_t*, uint32_t);
    int32_t (*p_set_gpr)(rust_spu_t*, uint32_t, const uint8_t*);
    int32_t (*p_set_pc)(rust_spu_t*, uint32_t);
    int32_t (*p_push_inmbox)(rust_spu_t*, uint32_t);
    int32_t (*p_pop_outmbox)(rust_spu_t*, uint32_t*);
    int32_t (*p_signal)(rust_spu_t*, uint32_t, uint32_t);
    rust_spu_outcome_t (*p_run)(rust_spu_t*, uint32_t, uint32_t*, uint32_t*);
    rust_spu_outcome_t (*p_step)(rust_spu_t*, uint32_t*);
    int32_t (*p_get_pc)(rust_spu_t*, uint32_t*);
    int32_t (*p_get_gpr)(rust_spu_t*, uint32_t, uint8_t*);
    int32_t (*p_get_ls)(rust_spu_t*, uint8_t*, uint32_t);
    int32_t (*p_get_park_pc)(rust_spu_t*, uint32_t*);
} rpcs3_spu_ffi_vtable;

static const rpcs3_spu_ffi_vtable VTABLE = {
    rust_spu_new,
    rust_spu_drop,
    rust_spu_load_ls,
    rust_spu_set_gpr,
    rust_spu_set_pc,
    rust_spu_push_inmbox,
    rust_spu_pop_outmbox,
    rust_spu_signal,
    rust_spu_run_until_event,
    rust_spu_step,
    rust_spu_get_pc,
    rust_spu_get_gpr,
    rust_spu_get_ls,
    rust_spu_get_park_pc,
};

/* Sanity-check the enum values. */
static const int OUTCOME_VALUES[] = {
    rust_spu_outcome_t_Continue,
    rust_spu_outcome_t_Stop,
    rust_spu_outcome_t_StallRead,
    rust_spu_outcome_t_StallWrite,
    rust_spu_outcome_t_Error,
};

/* Use VTABLE so the compiler can't optimize it out. */
const rpcs3_spu_ffi_vtable* rpcs3_spu_ffi_get_vtable(void) {
    return &VTABLE;
}

const int* rpcs3_spu_ffi_get_outcomes(void) {
    return OUTCOME_VALUES;
}
