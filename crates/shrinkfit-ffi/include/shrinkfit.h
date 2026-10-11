/* shrinkfit C ABI. SPDX-License-Identifier: Apache-2.0 */
#ifndef SHRINKFIT_H
#define SHRINKFIT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Library version, NUL-terminated, statically allocated. */
const char *shrinkfit_version(void);

/* Allocate / free buffers owned by the library. */
uint8_t *shrinkfit_alloc(size_t len);
void shrinkfit_free(uint8_t *ptr, size_t len);

/*
 * Shrink a UTF-8 JSON document.
 *
 *   input / input_len      the document
 *   options / options_len  a UTF-8 JSON object, e.g. {"max_chars":4000,"format":"yaml"}
 *   out_len                receives the length of the returned buffer
 *
 * Returns a UTF-8 JSON envelope (not NUL-terminated), never NULL:
 *   {"ok":true,"text":"...","report":{...}}   or   {"ok":false,"error":"..."}
 * Release it with shrinkfit_free(ptr, *out_len).
 */
uint8_t *shrinkfit_shrink(const uint8_t *input, size_t input_len,
                          const uint8_t *options, size_t options_len,
                          size_t *out_len);

#ifdef __cplusplus
}
#endif

#endif /* SHRINKFIT_H */
