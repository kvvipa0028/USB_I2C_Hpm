#ifndef SIGGEN_COMMAND_H
#define SIGGEN_COMMAND_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SIGGEN_LINE_CAP 160U

typedef enum {
    SIG_CMD_NONE = 0,
    SIG_CMD_START,
    SIG_CMD_STOP,
    SIG_CMD_STATUS,
    SIG_CMD_HELP,
    SIG_CMD_RELEASE,
    SIG_CMD_SLAVE,
    SIG_CMD_MASTER,
    SIG_CMD_STRETCH,
    SIG_CMD_NACK_ADDRESS,
    SIG_CMD_NACK_DATA,
    SIG_CMD_HOLD,
    SIG_CMD_TABLE_READ,
    SIG_CMD_TABLE_FILL,
    SIG_CMD_TABLE_WRITE
} siggen_command_kind_t;

typedef struct {
    siggen_command_kind_t kind;
    uint32_t value[2];
    uint8_t data[32];
    size_t data_len;
} siggen_command_t;

typedef enum {
    SIG_LINE_NONE = 0,
    SIG_LINE_READY,
    SIG_LINE_REJECT
} siggen_line_result_t;

typedef struct {
    char text[SIGGEN_LINE_CAP];
    size_t length;
    bool invalid;
    bool suppress_lf;
} siggen_line_assembler_t;

bool siggen_command_parse(const char *line, siggen_command_t *out);
void siggen_line_assembler_init(siggen_line_assembler_t *assembler);
siggen_line_result_t siggen_line_assembler_feed(siggen_line_assembler_t *assembler,
                                                unsigned char byte);

#endif
