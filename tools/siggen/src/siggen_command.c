#include "siggen_command.h"

static bool is_space(char c) { return c == ' ' || c == '\t'; }

static int text_compare(const char *a, const char *b)
{
    size_t i = 0U;
    while (a[i] == b[i] && a[i] != '\0') ++i;
    return (unsigned char)a[i] - (unsigned char)b[i];
}

static size_t text_length(const char *s)
{
    size_t n = 0U;
    while (s[n] != '\0') ++n;
    return n;
}

static bool next_token(const char **cursor, char *token, size_t capacity)
{
    const char *p = *cursor;
    size_t n = 0U;
    while (is_space(*p)) {
        ++p;
    }
    if (*p == '\0') {
        return false;
    }
    while (*p != '\0' && !is_space(*p)) {
        if ((unsigned char)*p < 0x20U || n + 1U >= capacity) {
            return false;
        }
        token[n++] = *p++;
    }
    token[n] = '\0';
    *cursor = p;
    return true;
}

static bool no_more_tokens(const char *cursor)
{
    while (is_space(*cursor)) {
        ++cursor;
    }
    return *cursor == '\0';
}

static int hex_value(char c)
{
    if (c >= '0' && c <= '9') return (int)(c - '0');
    if (c >= 'a' && c <= 'f') return (int)(c - 'a' + 10);
    if (c >= 'A' && c <= 'F') return (int)(c - 'A' + 10);
    return -1;
}

static bool parse_u32(const char *s, uint32_t *value)
{
    uint32_t v = 0U;
    size_t i = 0U;
    unsigned base = 10U;
    bool any = false;

    if (s[0] == '0' && s[1] == 'x') {
        base = 16U;
        i = 2U;
    } else if (s[0] == '0' && s[1] == 'X') {
        base = 16U;
        i = 2U;
    }
    for (; s[i] != '\0'; ++i) {
        int d = (base == 16U) ? hex_value(s[i]) :
                ((s[i] >= '0' && s[i] <= '9') ? (int)(s[i] - '0') : -1);
        if (d < 0 || (unsigned)d >= base || v > (UINT32_MAX - (uint32_t)d) / base) {
            return false;
        }
        v = v * base + (uint32_t)d;
        any = true;
    }
    if (!any) return false;
    *value = v;
    return true;
}

static bool parse_addr(const char *s, uint32_t *value)
{
    return parse_u32(s, value) && *value >= 0x08U && *value <= 0x77U;
}

static bool parse_hex_data(const char *s, uint8_t *data, size_t *length)
{
    size_t n = text_length(s);
    size_t i;
    if (n == 0U || n > 64U || (n & 1U) != 0U) return false;
    for (i = 0U; i < n / 2U; ++i) {
        int hi = hex_value(s[2U * i]);
        int lo = hex_value(s[2U * i + 1U]);
        if (hi < 0 || lo < 0) return false;
        data[i] = (uint8_t)((hi << 4) | lo);
    }
    *length = n / 2U;
    return true;
}

bool siggen_command_parse(const char *line, siggen_command_t *out)
{
    char t0[32], t1[80], t2[80];
    const char *p;
    siggen_command_t result = { 0 };
    uint32_t v;

    if (line == NULL || out == NULL) return false;
    p = line;
    if (!next_token(&p, t0, sizeof(t0))) return false;

    if (!text_compare(t0, "start")) result.kind = SIG_CMD_START;
    else if (!text_compare(t0, "stop")) result.kind = SIG_CMD_STOP;
    else if (!text_compare(t0, "status")) result.kind = SIG_CMD_STATUS;
    else if (!text_compare(t0, "help")) result.kind = SIG_CMD_HELP;
    else if (!text_compare(t0, "release")) result.kind = SIG_CMD_RELEASE;
    else if (!text_compare(t0, "slave") || !text_compare(t0, "master")) {
        if (!next_token(&p, t1, sizeof(t1)) || !next_token(&p, t2, sizeof(t2)) ||
            !parse_addr(t1, &result.value[0]) || !parse_u32(t2, &result.value[1]) ||
            (result.value[1] != 100000U && result.value[1] != 400000U && result.value[1] != 1000000U)) return false;
        result.kind = !text_compare(t0, "slave") ? SIG_CMD_SLAVE : SIG_CMD_MASTER;
    } else if (!text_compare(t0, "stretch")) {
        if (!next_token(&p, t1, sizeof(t1)) || !parse_u32(t1, &v) || v > 200000U) return false;
        result.kind = SIG_CMD_STRETCH; result.value[0] = v;
    } else if (!text_compare(t0, "nack")) {
        if (!next_token(&p, t1, sizeof(t1)) || !next_token(&p, t2, sizeof(t2)) || !parse_u32(t2, &v)) return false;
        if (!text_compare(t1, "address")) result.kind = SIG_CMD_NACK_ADDRESS;
        else if (!text_compare(t1, "data")) result.kind = SIG_CMD_NACK_DATA;
        else return false;
        if ((result.kind == SIG_CMD_NACK_ADDRESS && v > 1U) ||
            (result.kind == SIG_CMD_NACK_DATA && v > 256U)) return false;
        result.value[0] = v;
    } else if (!text_compare(t0, "hold")) {
        if (!next_token(&p, t1, sizeof(t1)) || !next_token(&p, t2, sizeof(t2)) || !parse_u32(t2, &v) || v < 1U || v > 5000U) return false;
        if (!text_compare(t1, "scl")) result.value[0] = 1U;
        else if (!text_compare(t1, "sda")) result.value[0] = 2U;
        else if (!text_compare(t1, "both")) result.value[0] = 3U;
        else return false;
        result.kind = SIG_CMD_HOLD; result.value[1] = v;
    } else if (!text_compare(t0, "table")) {
        if (!next_token(&p, t1, sizeof(t1))) return false;
        if (!text_compare(t1, "read")) {
            if (!next_token(&p, t1, sizeof(t1)) || !next_token(&p, t2, sizeof(t2)) || !parse_u32(t1, &result.value[0]) || !parse_u32(t2, &result.value[1]) || result.value[0] > 255U || result.value[1] < 1U || result.value[1] > 32U || result.value[0] + result.value[1] > 256U) return false;
            result.kind = SIG_CMD_TABLE_READ;
        } else if (!text_compare(t1, "fill")) {
            if (!next_token(&p, t1, sizeof(t1)) || !parse_u32(t1, &v) || v > 255U) return false;
            result.kind = SIG_CMD_TABLE_FILL; result.value[0] = v;
        } else if (!text_compare(t1, "write")) {
            if (!next_token(&p, t1, sizeof(t1)) || !next_token(&p, t2, sizeof(t2)) || !parse_u32(t1, &result.value[0]) || result.value[0] > 255U || !parse_hex_data(t2, result.data, &result.data_len) || result.value[0] + result.data_len > 256U) return false;
            result.kind = SIG_CMD_TABLE_WRITE;
        } else return false;
    } else return false;

    if (!no_more_tokens(p)) return false;
    *out = result;
    return true;
}

void siggen_line_assembler_init(siggen_line_assembler_t *assembler)
{
    if (assembler != NULL) {
        assembler->text[0] = '\0';
        assembler->length = 0U;
        assembler->invalid = false;
        assembler->suppress_lf = false;
    }
}

siggen_line_result_t siggen_line_assembler_feed(siggen_line_assembler_t *assembler, unsigned char byte)
{
    siggen_line_result_t result;
    if (assembler == NULL) return SIG_LINE_REJECT;
    if (assembler->suppress_lf && byte == '\n') { assembler->suppress_lf = false; return SIG_LINE_NONE; }
    assembler->suppress_lf = false;
    if (byte == '\r' || byte == '\n') {
        assembler->text[assembler->length] = '\0';
        result = assembler->invalid ? SIG_LINE_REJECT : SIG_LINE_READY;
        assembler->length = 0U; assembler->invalid = false; assembler->suppress_lf = byte == '\r';
        return result;
    }
    if ((byte < 0x20U && byte != '\t') || byte == 0x7fU || assembler->length + 1U >= SIGGEN_LINE_CAP) {
        assembler->invalid = true;
        return SIG_LINE_NONE;
    }
    assembler->text[assembler->length++] = (char)byte;
    return SIG_LINE_NONE;
}
