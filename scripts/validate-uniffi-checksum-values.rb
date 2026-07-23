#!/usr/bin/env ruby
# frozen_string_literal: true

require "json"

class ChecksumValidationError < StandardError; end

def fail_closed(message)
  raise ChecksumValidationError, message
end

def require_range(data, offset, length, label)
  fail_closed("#{label} is truncated") if offset.negative? || length.negative? || offset + length > data.bytesize
end

def u16le(data, offset, label)
  require_range(data, offset, 2, label)
  data.byteslice(offset, 2).unpack1("v")
end

def u32le(data, offset, label)
  require_range(data, offset, 4, label)
  data.byteslice(offset, 4).unpack1("V")
end

def u64le(data, offset, label)
  require_range(data, offset, 8, label)
  data.byteslice(offset, 8).unpack1("Q<")
end

def c_string(data, offset, limit, label)
  require_range(data, offset, 1, label)
  finish = data.index("\0", offset)
  fail_closed("#{label} has an unterminated string") unless finish && finish < limit

  data.byteslice(offset, finish - offset)
end

def expected_checksums(manifest_path)
  manifest = JSON.parse(File.read(manifest_path))
  entries = [manifest.fetch("version")] + manifest.fetch("functions")
  values = entries.to_h { |entry| [entry.fetch("name"), Integer(entry.fetch("checksum"))] }
  fail_closed("manifest has duplicate checksum names") unless values.length == entries.length
  values.each do |name, value|
    fail_closed("manifest checksum #{name} is outside uint16") unless (0..0xffff).cover?(value)
  end
  values
rescue Errno::ENOENT, JSON::ParserError, KeyError, TypeError, ArgumentError => error
  fail_closed("cannot read checksum manifest: #{error.message}")
end

def decode_checksum_body(data, offset, architecture, label, name, expected)
  case architecture
  when "arm64"
    instruction = u32le(data, offset, "#{label} #{name}")
    ret = u32le(data, offset + 4, "#{label} #{name}")
    fail_closed("#{label} checksum function #{name} has a noncanonical body") unless (instruction & 0xffe0001f) == 0x52800000 && ret == 0xd65f03c0
    actual = (instruction >> 5) & 0xffff
  when "armv7"
    first_half = u16le(data, offset, "#{label} #{name}")
    second_half = u16le(data, offset + 2, "#{label} #{name}")
    instruction = (first_half << 16) | second_half
    ret = u16le(data, offset + 4, "#{label} #{name}")
    fail_closed("#{label} checksum function #{name} has a noncanonical body") unless (instruction & 0xfbf08f00) == 0xf2400000 && ret == 0x4770
    actual = ((instruction >> 16) & 0xf) << 12
    actual |= ((instruction >> 26) & 1) << 11
    actual |= ((instruction >> 12) & 0x7) << 8
    actual |= instruction & 0xff
  when "x86", "x86_64"
    require_range(data, offset, 5, "#{label} #{name}")
    if data.getbyte(offset) == 0x66 && data.getbyte(offset + 1) == 0xb8 && data.getbyte(offset + 4) == 0xc3
      actual = u16le(data, offset + 2, "#{label} #{name}")
    else
      require_range(data, offset, 11, "#{label} #{name}")
      frame_pointer_return = [0x55, 0x48, 0x89, 0xe5, 0xb8]
      fail_closed("#{label} checksum function #{name} has a noncanonical body") unless data.byteslice(offset, 5).bytes == frame_pointer_return && data.getbyte(offset + 9) == 0x5d && data.getbyte(offset + 10) == 0xc3
      actual = u32le(data, offset + 5, "#{label} #{name}")
      fail_closed("#{label} checksum function #{name} returns a non-uint16 constant") unless actual <= 0xffff
    end
  else
    fail_closed("unsupported checksum architecture #{architecture}")
  end

  fail_closed("#{label} checksum mismatch for #{name}: expected #{expected}, found #{actual}") unless actual == expected
end

def elf_sections(data, elf_class, label)
  section_offset = elf_class == 1 ? u32le(data, 32, label) : u64le(data, 40, label)
  section_entry_size = u16le(data, elf_class == 1 ? 46 : 58, label)
  section_count = u16le(data, elf_class == 1 ? 48 : 60, label)
  expected_size = elf_class == 1 ? 40 : 64
  fail_closed("#{label} has an unsupported ELF section header layout") unless section_entry_size == expected_size && section_count.positive?

  section_count.times.map do |index|
    offset = section_offset + index * section_entry_size
    require_range(data, offset, section_entry_size, label)
    if elf_class == 1
      { type: u32le(data, offset + 4, label), address: u32le(data, offset + 12, label), offset: u32le(data, offset + 16, label), size: u32le(data, offset + 20, label), link: u32le(data, offset + 24, label), entry_size: u32le(data, offset + 36, label) }
    else
      { type: u32le(data, offset + 4, label), address: u64le(data, offset + 16, label), offset: u64le(data, offset + 24, label), size: u64le(data, offset + 32, label), link: u32le(data, offset + 40, label), entry_size: u64le(data, offset + 56, label) }
    end
  end
end

def elf_symbol_values(data, elf_class, sections, expected, label)
  symbol_sections = sections.each_index.select { |index| sections[index][:type] == 11 }
  fail_closed("#{label} must contain exactly one ELF dynamic symbol table") unless symbol_sections.length == 1
  symbols = sections.fetch(symbol_sections.fetch(0))
  expected_entry_size = elf_class == 1 ? 16 : 24
  fail_closed("#{label} has an unsupported ELF dynamic symbol layout") unless symbols[:entry_size] == expected_entry_size && (symbols[:size] % symbols[:entry_size]).zero?
  string_table = sections[symbols[:link]]
  fail_closed("#{label} has no ELF dynamic symbol string table") unless string_table
  require_range(data, string_table[:offset], string_table[:size], label)

  found = Hash.new { |hash, key| hash[key] = [] }
  (symbols[:size] / symbols[:entry_size]).times do |index|
    offset = symbols[:offset] + index * symbols[:entry_size]
    require_range(data, offset, symbols[:entry_size], label)
    name_offset = u32le(data, offset, label)
    name = c_string(data, string_table[:offset] + name_offset, string_table[:offset] + string_table[:size], label)
    next unless name.start_with?("uniffi_editor_core_checksum_func_")
    name = name.delete_prefix("uniffi_editor_core_checksum_func_")
    next unless expected.key?(name)

    if elf_class == 1
      info = data.getbyte(offset + 12)
      section_index = u16le(data, offset + 14, label)
      value = u32le(data, offset + 4, label)
    else
      info = data.getbyte(offset + 4)
      section_index = u16le(data, offset + 6, label)
      value = u64le(data, offset + 8, label)
    end
    fail_closed("#{label} checksum symbol #{name} is not a defined function") unless (info & 0x0f) == 2 && section_index.positive?
    found[name] << value
  end

  expected.each_key do |name|
    fail_closed("#{label} is missing checksum symbol #{name}") unless found.key?(name)
    fail_closed("#{label} has duplicate checksum symbol #{name}") unless found[name].length == 1
  end
  found.transform_values(&:first)
end

def elf_load_segments(data, elf_class, label)
  program_offset = elf_class == 1 ? u32le(data, 28, label) : u64le(data, 32, label)
  program_entry_size = u16le(data, elf_class == 1 ? 42 : 54, label)
  program_count = u16le(data, elf_class == 1 ? 44 : 56, label)
  expected_size = elf_class == 1 ? 32 : 56
  fail_closed("#{label} has an unsupported ELF program header layout") unless program_entry_size == expected_size && program_count.positive?

  segments = []
  program_count.times do |index|
    offset = program_offset + index * program_entry_size
    require_range(data, offset, program_entry_size, label)
    next unless u32le(data, offset, label) == 1

    if elf_class == 1
      segments << { offset: u32le(data, offset + 4, label), address: u32le(data, offset + 8, label), size: u32le(data, offset + 16, label) }
    else
      segments << { offset: u64le(data, offset + 8, label), address: u64le(data, offset + 16, label), size: u64le(data, offset + 32, label) }
    end
  end
  segments
end

def elf_file_offset(segments, address, label, name)
  segment = segments.find { |entry| address >= entry[:address] && address < entry[:address] + entry[:size] }
  fail_closed("#{label} checksum function #{name} is outside a loadable segment") unless segment
  segment[:offset] + address - segment[:address]
end

def validate_elf(path, abi, expected, label)
  data = File.binread(path)
  fail_closed("#{label} is not an ELF file") unless data.start_with?("\x7fELF")
  elf_class = data.getbyte(4)
  fail_closed("#{label} has an unsupported ELF class") unless [1, 2].include?(elf_class)
  fail_closed("#{label} is not little-endian ELF") unless data.getbyte(5) == 1
  machine = u16le(data, 18, label)
  expected_machine = { "arm64-v8a" => [2, 183, "arm64"], "armeabi-v7a" => [1, 40, "armv7"], "x86" => [1, 3, "x86"], "x86_64" => [2, 62, "x86_64"] }.fetch(abi) { fail_closed("unknown Android ABI #{abi}") }
  fail_closed("#{label} has the wrong ELF machine type") unless elf_class == expected_machine[0] && machine == expected_machine[1]

  sections = elf_sections(data, elf_class, label)
  values = elf_symbol_values(data, elf_class, sections, expected, label)
  segments = elf_load_segments(data, elf_class, label)
  expected.each { |name, checksum| decode_checksum_body(data, elf_file_offset(segments, values.fetch(name) & ~1, label, name), expected_machine[2], label, name, checksum) }
end

def macho_object_symbols(data, architecture, expected, label)
  fail_closed("#{label} is not a 64-bit little-endian Mach-O object") unless u32le(data, 0, label) == 0xfeedfacf
  cpu_type = u32le(data, 4, label)
  expected_cpu = { "arm64" => 0x0100000c, "x86_64" => 0x01000007 }.fetch(architecture) { fail_closed("unsupported iOS architecture #{architecture}") }
  fail_closed("#{label} has the wrong Mach-O CPU type") unless cpu_type == expected_cpu
  command_count = u32le(data, 16, label)
  command_size = u32le(data, 20, label)
  command_offset = 32
  require_range(data, command_offset, command_size, label)
  command_limit = command_offset + command_size
  sections = []
  symbol_table = nil

  command_count.times do
    fail_closed("#{label} has a truncated Mach-O load command header") if command_offset + 8 > command_limit
    command = u32le(data, command_offset, label)
    size = u32le(data, command_offset + 4, label)
    fail_closed("#{label} has an invalid Mach-O load command") if size < 8
    fail_closed("#{label} has a load command outside its declared command area") if command_offset + size > command_limit
    if command == 0x19
      fail_closed("#{label} has an LC_SEGMENT_64 command smaller than 72 bytes") if size < 72
      section_count = u32le(data, command_offset + 64, label)
      fail_closed("#{label} has an invalid LC_SEGMENT_64 section layout") unless size == 72 + section_count * 80
      section_count.times do |index|
        offset = command_offset + 72 + index * 80
        sections << { address: u64le(data, offset + 32, label), size: u64le(data, offset + 40, label), offset: u32le(data, offset + 48, label) }
      end
    elsif command == 0x2
      fail_closed("#{label} has an LC_SYMTAB command whose size is not 24 bytes") unless size == 24
      fail_closed("#{label} has duplicate Mach-O symbol tables") if symbol_table
      symbol_table = { offset: u32le(data, command_offset + 8, label), count: u32le(data, command_offset + 12, label), strings: u32le(data, command_offset + 16, label), string_size: u32le(data, command_offset + 20, label) }
    end
    command_offset += size
  end
  fail_closed("#{label} has an incomplete Mach-O load command area") unless command_offset == command_limit
  return {} unless symbol_table
  require_range(data, symbol_table[:offset], symbol_table[:count] * 16, label)
  require_range(data, symbol_table[:strings], symbol_table[:string_size], label)

  found = Hash.new { |hash, key| hash[key] = [] }
  symbol_table[:count].times do |index|
    offset = symbol_table[:offset] + index * 16
    name = c_string(data, symbol_table[:strings] + u32le(data, offset, label), symbol_table[:strings] + symbol_table[:string_size], label)
    next unless name.start_with?("_uniffi_editor_core_checksum_func_")
    name = name.delete_prefix("_uniffi_editor_core_checksum_func_")
    next unless expected.key?(name)

    type = data.getbyte(offset + 4)
    section_index = data.getbyte(offset + 5)
    next unless (type & 0x0e) == 0x0e
    fail_closed("#{label} checksum symbol #{name} has no section") if section_index.zero? || section_index > sections.length
    section = sections.fetch(section_index - 1)
    value = u64le(data, offset + 8, label)
    fail_closed("#{label} checksum symbol #{name} is outside its section") if value < section[:address] || value >= section[:address] + section[:size]
    found[name] << section[:offset] + value - section[:address]
  end
  found
end

def validate_macho(objects, architecture, expected, label)
  found = Hash.new { |hash, key| hash[key] = [] }
  objects.each do |path|
    data = File.binread(path)
    macho_object_symbols(data, architecture, expected, "#{label} object #{File.basename(path)}").each do |name, offsets|
      offsets.each { |offset| found[name] << [data, offset] }
    end
  end
  expected.each do |name, checksum|
    fail_closed("#{label} is missing checksum symbol #{name}") unless found.key?(name)
    fail_closed("#{label} has duplicate checksum symbol #{name}") unless found[name].length == 1
    data, offset = found[name].fetch(0)
    decode_checksum_body(data, offset, architecture, label, name, checksum)
  end
end

def parse_arguments(argv)
  fail_closed("usage: validate-uniffi-checksum-values.rb --manifest PATH --label LABEL (--elf ABI PATH | --macho ARCH OBJECT...)") unless argv.length >= 6
  manifest = argv.shift
  manifest_path = argv.shift
  label = argv.shift
  label_text = argv.shift
  fail_closed("missing --manifest or --label") unless manifest == "--manifest" && manifest_path && label == "--label" && label_text
  format = argv.shift
  architecture = argv.shift
  fail_closed("missing native checksum input") unless architecture && !argv.empty?
  [manifest_path, label_text, format, architecture, argv]
end

begin
  manifest_path, label, format, architecture, paths = parse_arguments(ARGV.dup)
  expected = expected_checksums(manifest_path)
  case format
  when "--elf"
    fail_closed("ELF validation accepts exactly one library") unless paths.length == 1
    validate_elf(paths.fetch(0), architecture, expected, label)
  when "--macho"
    validate_macho(paths, architecture, expected, label)
  else
    fail_closed("unknown native checksum format #{format}")
  end
rescue ChecksumValidationError, Errno::ENOENT => error
  warn "checksum parser: #{error.message}"
  exit 1
end
