package mas

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"reflect"
	"regexp"
	"strconv"
	"strings"
	"unicode/utf8"
)

const MaxFrameBytes = 65536
const maxJSONDepth = 32

var integer = regexp.MustCompile(`^-?(0|[1-9][0-9]*)$`)

// ValidateObject enforces the native contract before values cross the boundary.
// Numbers stay json.Number throughout validation; no float64 conversion occurs.
func ValidateObject(raw []byte) error {
	if len(raw) == 0 || len(raw) > MaxFrameBytes {
		return fmt.Errorf("JSON size must be 1..%d bytes", MaxFrameBytes)
	}
	if !utf8.Valid(raw) {
		return fmt.Errorf("JSON contains invalid UTF-8")
	}
	if err := validateSurrogates(raw); err != nil {
		return err
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	first, err := d.Token()
	if err != nil || first != json.Delim('{') {
		return fmt.Errorf("native values must be JSON objects")
	}
	if err := validateValue(d, first, 1); err != nil {
		return err
	}
	if _, err := d.Token(); err != io.EOF {
		return fmt.Errorf("unexpected trailing JSON")
	}
	return nil
}

func validateValue(d *json.Decoder, token json.Token, depth int) error {
	switch value := token.(type) {
	case json.Number:
		if !integer.MatchString(string(value)) || len(strings.TrimPrefix(string(value), "-")) > 1024 {
			return fmt.Errorf("native JSON requires integer numbers: %s", value)
		}
	case json.Delim:
		if depth > maxJSONDepth {
			return fmt.Errorf("JSON nesting exceeds %d", maxJSONDepth)
		}
		if value == '{' {
			seen := make(map[string]bool)
			for d.More() {
				keyToken, err := d.Token()
				if err != nil {
					return err
				}
				key, ok := keyToken.(string)
				if !ok {
					return fmt.Errorf("JSON object key is not a string")
				}
				if seen[key] {
					return fmt.Errorf("duplicate JSON key %q", key)
				}
				seen[key] = true
				child, err := d.Token()
				if err != nil {
					return err
				}
				if err := validateValue(d, child, depth+1); err != nil {
					return err
				}
			}
			end, err := d.Token()
			if err != nil || end != json.Delim('}') {
				return fmt.Errorf("unclosed JSON object")
			}
		} else if value == '[' {
			for d.More() {
				child, err := d.Token()
				if err != nil {
					return err
				}
				if err := validateValue(d, child, depth+1); err != nil {
					return err
				}
			}
			end, err := d.Token()
			if err != nil || end != json.Delim(']') {
				return fmt.Errorf("unclosed JSON array")
			}
		} else {
			return fmt.Errorf("unexpected JSON delimiter")
		}
	}
	return nil
}

// encoding/json replaces unpaired surrogate escapes. Reject them instead.
func validateSurrogates(raw []byte) error {
	inString := false
	for i := 0; i < len(raw); i++ {
		if raw[i] == '"' {
			inString = !inString
			continue
		}
		if !inString || raw[i] != '\\' {
			continue
		}
		i++
		if i >= len(raw) {
			return fmt.Errorf("unfinished JSON escape")
		}
		if raw[i] != 'u' {
			continue
		}
		if i+4 >= len(raw) {
			return fmt.Errorf("unfinished Unicode escape")
		}
		code, err := strconv.ParseUint(string(raw[i+1:i+5]), 16, 16)
		if err != nil {
			return fmt.Errorf("invalid Unicode escape")
		}
		i += 4
		if code >= 0xDC00 && code <= 0xDFFF {
			return fmt.Errorf("unpaired low surrogate")
		}
		if code < 0xD800 || code > 0xDBFF {
			continue
		}
		if i+6 >= len(raw) || raw[i+1] != '\\' || raw[i+2] != 'u' {
			return fmt.Errorf("unpaired high surrogate")
		}
		low, err := strconv.ParseUint(string(raw[i+3:i+7]), 16, 16)
		if err != nil || low < 0xDC00 || low > 0xDFFF {
			return fmt.Errorf("unpaired high surrogate")
		}
		i += 6
	}
	return nil
}

func object(value any) (json.RawMessage, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, err
	}
	if err := validateNativeValues(reflect.ValueOf(value), 0); err != nil {
		return nil, err
	}
	if err := ValidateObject(raw); err != nil {
		return nil, err
	}
	return raw, nil
}

// Reject types encoding/json would silently coerce (integral floats and invalid
// UTF-8 strings). Explicit MarshalJSON codecs remain supported and their bytes
// are checked by ValidateObject. Marshal above detects cycles first.
func validateNativeValues(value reflect.Value, depth int) error {
	if !value.IsValid() {
		return nil
	}
	if depth > 64 {
		return fmt.Errorf("native value nesting too deep")
	}
	if value.CanInterface() {
		if _, ok := value.Interface().(json.Marshaler); ok {
			return nil
		}
	}
	switch value.Kind() {
	case reflect.Float32, reflect.Float64:
		return fmt.Errorf("native values require integers; encode decimals explicitly as scaled integers or strings")
	case reflect.String:
		if !utf8.ValidString(value.String()) {
			return fmt.Errorf("native string contains invalid UTF-8")
		}
	case reflect.Interface, reflect.Pointer:
		if !value.IsNil() {
			return validateNativeValues(value.Elem(), depth+1)
		}
	case reflect.Map:
		iterator := value.MapRange()
		for iterator.Next() {
			if err := validateNativeValues(iterator.Key(), depth+1); err != nil {
				return err
			}
			if err := validateNativeValues(iterator.Value(), depth+1); err != nil {
				return err
			}
		}
	case reflect.Array, reflect.Slice:
		for i := 0; i < value.Len(); i++ {
			if err := validateNativeValues(value.Index(i), depth+1); err != nil {
				return err
			}
		}
	case reflect.Struct:
		for i := 0; i < value.NumField(); i++ {
			field := value.Type().Field(i)
			if field.PkgPath != "" || strings.Split(field.Tag.Get("json"), ",")[0] == "-" {
				continue
			}
			if err := validateNativeValues(value.Field(i), depth+1); err != nil {
				return err
			}
		}
	}
	return nil
}

// Decode keeps integer values exact when T contains interface{} fields.
func Decode[T any](raw json.RawMessage) (T, error) {
	var result T
	if err := ValidateObject(raw); err != nil {
		return result, err
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if err := d.Decode(&result); err != nil {
		return result, err
	}
	if err := validateNativeValues(reflect.ValueOf(result), 0); err != nil {
		var empty T
		return empty, err
	}
	return result, nil
}
