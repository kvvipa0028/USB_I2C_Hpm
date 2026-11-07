#ifndef SIGGEN_SESSION_H
#define SIGGEN_SESSION_H
#include "siggen_peer.h"
#define SIGGEN_RESPONSE_CAP 1024U
enum siggen_session_event {
    SIG_SESSION_NONE, SIG_SESSION_COMMAND, SIG_SESSION_STOP,
    SIG_SESSION_STOP_ERROR, SIG_SESSION_ERROR
};
/* Parsing touches only the main-loop line state. STOP events must be applied
 * while serialized with the peer ISR; replies may be formatted after unlock. */
enum siggen_session_event siggen_session_parse(siggen_line_assembler_t *line,
    int byte, siggen_command_t *command, const char **error);
/* Convenience path for callers without concurrent ISR access. */
bool siggen_session_feed(siggen_peer_t *peer, siggen_line_assembler_t *line,
                          int byte, uint64_t now, char out[SIGGEN_RESPONSE_CAP]);
#endif
