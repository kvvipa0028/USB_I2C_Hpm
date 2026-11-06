#include "siggen_session.h"
#include <stdio.h>
enum siggen_session_event siggen_session_parse(siggen_line_assembler_t *line,
    int byte, siggen_command_t *command, const char **error)
{
    *error = NULL;
    if (byte == -2) {
        siggen_line_assembler_init(line);
        *error = "ERR rx overflow; stopped\n";
        return SIG_SESSION_STOP_ERROR;
    }
    if (byte < 0) return SIG_SESSION_NONE;
    bool invalid_before = line->invalid;
    siggen_line_result_t event = siggen_line_assembler_feed(line, (unsigned char)byte);
    if (line->invalid && !invalid_before) return SIG_SESSION_STOP;
    if (event == SIG_LINE_REJECT) {
        *error = "ERR malformed line; stopped\n";
        return SIG_SESSION_ERROR;
    }
    if (event != SIG_LINE_READY || line->text[0] == '\0') return SIG_SESSION_NONE;
    if (!siggen_command_parse(line->text, command)) {
        *error = "ERR command\n";
        return SIG_SESSION_ERROR;
    }
    return SIG_SESSION_COMMAND;
}
bool siggen_session_feed(siggen_peer_t *p, siggen_line_assembler_t *line,
                          int byte, uint64_t now, char out[SIGGEN_RESPONSE_CAP])
{
    siggen_command_t command;
    const char *error;
    enum siggen_session_event event = siggen_session_parse(line, byte, &command, &error);
    if (event == SIG_SESSION_STOP || event == SIG_SESSION_STOP_ERROR) siggen_peer_stop(p);
    if (event == SIG_SESSION_COMMAND) {
        siggen_peer_command(p, &command, now, out, SIGGEN_RESPONSE_CAP);
        return true;
    }
    if (error != NULL) {
        (void)snprintf(out, SIGGEN_RESPONSE_CAP, "%s", error);
        return true;
    }
    return false;
}
