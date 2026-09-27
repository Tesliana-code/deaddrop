// Go observations for the Envelope V0 wire conformance corpus.
//
// Profiles:
//
//	go-default     encoding/json (v1) Marshal / Unmarshal + contract-level
//	               validation. Ordinary library use. No byte comparison.
//	go-v2-default  encoding/json/v2 Marshal / Unmarshal + contract-level
//	               validation. No byte comparison.
//	go-tuned       v1 Encoder with SetEscapeHTML(false), trailing newline
//	               trimmed, DisallowUnknownFields, and the PROTOCOL.md
//	               re-encode byte-equality acceptance rule.
//	go-strict      hand-written encoder from RULES.md (no JSON encoder used),
//	               ordinary v1 parsing, draft identifier profile, and
//	               re-encode byte equality.
//
// Emits one JSON line per (profile, vector) and per escape probe on stdout.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	jsonv2 "encoding/json/v2"
	"errors"
	"fmt"
	"os"
	"regexp"
	"unicode/utf8"
)

type envelope struct {
	Protocol      string   `json:"protocol"`
	ID            string   `json:"id"`
	From          string   `json:"from"`
	To            string   `json:"to"`
	Kind          string   `json:"kind"`
	CorrelationID *string  `json:"correlation_id"`
	Body          string   `json:"body"`
	ArtifactRefs  []string `json:"artifact_refs"`
}

var kinds = map[string]bool{
	"message": true, "request": true, "response": true, "handoff": true,
	"task_claim": true, "checkpoint": true, "acknowledgment": true,
	"error": true, "capability_declaration": true,
}

var refPattern = regexp.MustCompile(`^sha256:[0-9a-f]{64}$`)

// RULES.md R-ID: Unicode White_Space (identical to Rust char::is_whitespace).
func isWhiteSpace(r rune) bool {
	switch {
	case r >= 0x09 && r <= 0x0D, r == 0x20, r == 0x85, r == 0xA0, r == 0x1680:
		return true
	case r >= 0x2000 && r <= 0x200A:
		return true
	case r == 0x2028, r == 0x2029, r == 0x202F, r == 0x205F, r == 0x3000:
		return true
	}
	return false
}

func validIDContract(s string) bool { return s != "" }

func validIDStrict(s string) bool {
	if s == "" || !utf8.ValidString(s) {
		return false
	}
	first, _ := utf8.DecodeRuneInString(s)
	last, _ := utf8.DecodeLastRuneInString(s)
	if isWhiteSpace(first) || isWhiteSpace(last) {
		return false
	}
	for _, r := range s {
		if r < 0x20 || (r >= 0x7F && r <= 0x9F) {
			return false
		}
	}
	return true
}

func validate(e *envelope, validID func(string) bool) error {
	if e.Protocol != "deaddrop/0" {
		return errors.New("protocol")
	}
	for name, v := range map[string]string{"id": e.ID, "from": e.From, "to": e.To} {
		if !validID(v) {
			return fmt.Errorf("invalid %s", name)
		}
	}
	if !kinds[e.Kind] {
		return errors.New("kind")
	}
	if e.CorrelationID != nil && !validID(*e.CorrelationID) {
		return errors.New("invalid correlation_id")
	}
	for _, r := range e.ArtifactRefs {
		if !refPattern.MatchString(r) {
			return errors.New("artifact_refs")
		}
	}
	return nil
}

// ------------------------------------------------------------ encoders

func encodeDefault(e *envelope) ([]byte, error) { return json.Marshal(e) }

func encodeV2(e *envelope) ([]byte, error) { return jsonv2.Marshal(e) }

func encodeTuned(e *envelope) ([]byte, error) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(e); err != nil {
		return nil, err
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte{'\n'}), nil
}

const hexDigits = "0123456789abcdef"

func strictString(buf []byte, s string) ([]byte, error) {
	if !utf8.ValidString(s) {
		return nil, errors.New("string is not valid UTF-8")
	}
	buf = append(buf, '"')
	for i, r := range s {
		switch {
		case r == '"':
			buf = append(buf, '\\', '"')
		case r == '\\':
			buf = append(buf, '\\', '\\')
		case r == 0x08:
			buf = append(buf, '\\', 'b')
		case r == 0x09:
			buf = append(buf, '\\', 't')
		case r == 0x0A:
			buf = append(buf, '\\', 'n')
		case r == 0x0C:
			buf = append(buf, '\\', 'f')
		case r == 0x0D:
			buf = append(buf, '\\', 'r')
		case r < 0x20:
			buf = append(buf, '\\', 'u', '0', '0', hexDigits[r>>4], hexDigits[r&15])
		default:
			buf = append(buf, s[i:i+utf8.RuneLen(r)]...)
		}
	}
	return append(buf, '"'), nil
}

func encodeStrict(e *envelope) ([]byte, error) {
	var err error
	buf := []byte(`{"protocol":`)
	str := func(s string) {
		if err == nil {
			buf, err = strictString(buf, s)
		}
	}
	str(e.Protocol)
	buf = append(buf, `,"id":`...)
	str(e.ID)
	buf = append(buf, `,"from":`...)
	str(e.From)
	buf = append(buf, `,"to":`...)
	str(e.To)
	buf = append(buf, `,"kind":`...)
	str(e.Kind)
	buf = append(buf, `,"correlation_id":`...)
	if e.CorrelationID == nil {
		buf = append(buf, "null"...)
	} else {
		str(*e.CorrelationID)
	}
	buf = append(buf, `,"body":`...)
	str(e.Body)
	buf = append(buf, `,"artifact_refs":[`...)
	for i, r := range e.ArtifactRefs {
		if i > 0 {
			buf = append(buf, ',')
		}
		str(r)
	}
	buf = append(buf, "]}"...)
	return buf, err
}

// ------------------------------------------------------------ decoders

func decodeDefault(wire []byte) (*envelope, error) {
	var e envelope
	if err := json.Unmarshal(wire, &e); err != nil {
		return nil, err
	}
	return &e, validate(&e, validIDContract)
}

func decodeV2(wire []byte) (*envelope, error) {
	var e envelope
	if err := jsonv2.Unmarshal(wire, &e); err != nil {
		return nil, err
	}
	return &e, validate(&e, validIDContract)
}

func decodeWith(wire []byte, validID func(string) bool, encode func(*envelope) ([]byte, error)) (*envelope, error) {
	var e envelope
	dec := json.NewDecoder(bytes.NewReader(wire))
	dec.DisallowUnknownFields()
	if err := dec.Decode(&e); err != nil {
		return nil, err
	}
	if err := validate(&e, validID); err != nil {
		return nil, err
	}
	again, err := encode(&e)
	if err != nil {
		return nil, err
	}
	if !bytes.Equal(again, wire) {
		return nil, errors.New("non-canonical")
	}
	return &e, nil
}

func decodeTuned(wire []byte) (*envelope, error) {
	return decodeWith(wire, validIDContract, encodeTuned)
}

func decodeStrict(wire []byte) (*envelope, error) {
	if !utf8.Valid(wire) {
		return nil, errors.New("invalid UTF-8")
	}
	return decodeWith(wire, validIDStrict, encodeStrict)
}

type profile struct {
	name   string
	encode func(*envelope) ([]byte, error)
	decode func([]byte) (*envelope, error)
}

var profiles = []profile{
	{"go-default", encodeDefault, decodeDefault},
	{"go-v2-default", encodeV2, decodeV2},
	{"go-tuned", encodeTuned, decodeTuned},
	{"go-strict", encodeStrict, decodeStrict},
}

// ------------------------------------------------------------ harness

type fieldsHex struct {
	ID            string   `json:"id"`
	From          string   `json:"from"`
	To            string   `json:"to"`
	Kind          string   `json:"kind"`
	CorrelationID *string  `json:"correlation_id"`
	Body          string   `json:"body"`
	ArtifactRefs  []string `json:"artifact_refs"`
}

type vector struct {
	ID      string     `json:"id"`
	Class   string     `json:"class"`
	Fields  *fieldsHex `json:"fields_utf8_hex"`
	WireHex string     `json:"wire_hex"`
}

type corpus struct {
	Golden     map[string]any `json:"golden_fields"`
	Codepoints []rune         `json:"escape_probe_codepoints"`
	Vectors    []vector       `json:"vectors"`
}

func unhex(s string) string {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return string(b)
}

func fromHex(f *fieldsHex) *envelope {
	e := &envelope{
		Protocol: "deaddrop/0", ID: unhex(f.ID), From: unhex(f.From), To: unhex(f.To),
		Kind: unhex(f.Kind), Body: unhex(f.Body), ArtifactRefs: []string{},
	}
	if f.CorrelationID != nil {
		c := unhex(*f.CorrelationID)
		e.CorrelationID = &c
	}
	for _, r := range f.ArtifactRefs {
		e.ArtifactRefs = append(e.ArtifactRefs, unhex(r))
	}
	return e
}

func same(a, b *envelope) bool {
	if (a.CorrelationID == nil) != (b.CorrelationID == nil) {
		return false
	}
	if a.CorrelationID != nil && *a.CorrelationID != *b.CorrelationID {
		return false
	}
	if len(a.ArtifactRefs) != len(b.ArtifactRefs) {
		return false
	}
	for i := range a.ArtifactRefs {
		if a.ArtifactRefs[i] != b.ArtifactRefs[i] {
			return false
		}
	}
	return a.ID == b.ID && a.From == b.From && a.To == b.To && a.Kind == b.Kind && a.Body == b.Body
}

func bodyLiteral(wire []byte) []byte {
	start := bytes.Index(wire, []byte(`"body":`)) + len(`"body":`)
	end := bytes.Index(wire, []byte(`"artifact_refs"`))
	end = bytes.LastIndexByte(wire[:end], ',')
	return wire[start:end]
}

func emit(record map[string]any) {
	out, err := json.Marshal(record)
	if err != nil {
		panic(err)
	}
	fmt.Println(string(out))
}

func main() {
	raw, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var c corpus
	if err := json.Unmarshal(raw, &c); err != nil {
		panic(err)
	}

	for _, p := range profiles {
		for _, v := range c.Vectors {
			wire, _ := hex.DecodeString(v.WireHex)
			var expected *envelope
			encodeResult, encodedHex := "n/a", any(nil)
			if v.Class == "positive" {
				expected = fromHex(v.Fields)
				got, err := p.encode(expected)
				switch {
				case err != nil:
					encodeResult, encodedHex = "error", err.Error()
				case bytes.Equal(got, wire):
					encodeResult = "match"
				default:
					encodeResult, encodedHex = "mismatch", hex.EncodeToString(got)
				}
			}
			decodeResult, detail, fieldsMatch := "accept", "", any(nil)
			if got, err := p.decode(wire); err != nil {
				decodeResult, detail = "reject", err.Error()
			} else if expected != nil {
				fieldsMatch = same(got, expected)
			}
			var pass bool
			if expected != nil {
				pass = encodeResult == "match" && decodeResult == "accept" && fieldsMatch == true
			} else {
				pass = decodeResult == "reject"
			}
			emit(map[string]any{
				"type": "vector", "impl": p.name, "vector": v.ID, "class": v.Class,
				"encode": encodeResult, "encoded_hex": encodedHex, "decode": decodeResult,
				"fields_match": fieldsMatch, "detail": detail, "pass": pass,
			})
		}
		for _, cp := range c.Codepoints {
			e := &envelope{
				Protocol: "deaddrop/0", ID: c.Golden["id"].(string), From: c.Golden["from"].(string),
				To: c.Golden["to"].(string), Kind: "message", Body: string(cp), ArtifactRefs: []string{},
			}
			got, err := p.encode(e)
			if err != nil {
				emit(map[string]any{"type": "escape", "impl": p.name, "codepoint": cp, "literal_hex": nil, "error": err.Error()})
				continue
			}
			emit(map[string]any{"type": "escape", "impl": p.name, "codepoint": cp,
				"literal_hex": hex.EncodeToString(bodyLiteral(got)), "error": nil})
		}
	}

	// Supplementary probe: a nil slice, which is what a Go value built without
	// explicit initialisation holds when there are no artifact refs.
	nilRefs := &envelope{Protocol: "deaddrop/0", ID: "m", From: "a", To: "b", Kind: "message"}
	for _, p := range profiles {
		got, err := p.encode(nilRefs)
		emit(map[string]any{"type": "probe", "impl": p.name, "probe": "nil-artifact-refs-slice",
			"output": string(got), "error": fmt.Sprint(err)})
	}
}
