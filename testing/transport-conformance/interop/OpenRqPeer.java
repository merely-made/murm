// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

import net.fec.openrq.ArrayDataDecoder;
import net.fec.openrq.ArrayDataEncoder;
import net.fec.openrq.EncodingPacket;
import net.fec.openrq.OpenRQ;
import net.fec.openrq.Parsed;
import net.fec.openrq.parameters.FECParameters;

/** Independent OpenRQ peer for the bounded, single-symbol Al=1/N=1 profile. */
public final class OpenRqPeer {
    private static final int MAX_REQUEST = 512 * 1024;
    private static final int MAX_PACKETS = 1024;

    private OpenRqPeer() {}

    public static void main(String[] arguments) {
        try {
            require(arguments.length == 0, "peer accepts binary stdin only");
            byte[] request = readRequest();
            byte[] response = run(request);
            System.out.write(response);
            System.out.flush();
        }
        catch (Exception error) {
            System.err.println("OpenRqPeer: " + error.getMessage());
            System.exit(1);
        }
    }

    private static byte[] readRequest() throws Exception {
        ByteArrayOutputStream data = new ByteArrayOutputStream();
        byte[] chunk = new byte[8192];
        int length;
        while ((length = System.in.read(chunk)) != -1) {
            require(data.size() + length <= MAX_REQUEST, "request exceeds 512 KiB");
            data.write(chunk, 0, length);
        }
        return data.toByteArray();
    }

    private static byte[] run(byte[] request) throws Exception {
        ByteBuffer input = ByteBuffer.wrap(request);
        require(Arrays.equals(read(input, 4), new byte[] {'R', 'Q', 'I', '1'}), "request magic");
        int command = u8(input);
        require(command == 'E' || command == 'D', "unknown command");
        byte[] oti = read(input, 12);
        ByteBuffer fields = ByteBuffer.wrap(oti);
        long transferLength = 0;
        for (int index = 0; index < 5; index++) transferLength = (transferLength << 8) | u8(fields);
        u8(fields); // RFC6330 reserved octet is ignored by OpenRQ.
        int symbolSize = u16(fields);
        int sourceBlocks = u8(fields);
        int subBlocks = u16(fields);
        int alignment = u8(fields);
        require(transferLength >= 1 && transferLength <= 65536, "bounded F");
        require(symbolSize >= 1 && symbolSize <= 256, "bounded T");
        require(sourceBlocks >= 1 && sourceBlocks <= 16, "bounded Z");
        require(subBlocks == 1 && alignment == 1, "shared profile requires N=1, Al=1");
        long symbols = (transferLength + symbolSize - 1) / symbolSize;
        require(sourceBlocks <= symbols, "empty source block");
        require((symbols + sourceBlocks - 1) / sourceBlocks <= 256, "bounded block K");

        byte[] object = null;
        List<byte[]> packets = new ArrayList<byte[]>();
        if (command == 'E') {
            require(input.remaining() >= 4, "missing object length");
            long objectLength = Integer.toUnsignedLong(input.getInt());
            require(objectLength == transferLength, "object length differs from OTI F");
            object = read(input, (int)objectLength);
            int count = u16(input);
            require(count <= MAX_PACKETS, "too many requested packets");
            for (int index = 0; index < count; index++) {
                byte[] id = read(input, 4);
                validateId(id, sourceBlocks);
                packets.add(id);
            }
        }
        else {
            int count = u16(input);
            require(count <= MAX_PACKETS, "too many delivered packets");
            for (int index = 0; index < count; index++) {
                int length = u16(input);
                require(length == 4 + symbolSize, "profile requires one full symbol per packet");
                byte[] packet = read(input, length);
                validateId(packet, sourceBlocks);
                packets.add(packet);
            }
        }
        require(!input.hasRemaining(), "trailing request bytes");
        // Every allocation/work bound and every packet is checked before entering the codec.
        Parsed<FECParameters> parsed = FECParameters.parse(oti);
        require(parsed.isValid(), "OpenRQ rejected OTI: " + parsed.failureReason());
        FECParameters parameters = parsed.value();
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        DataOutputStream wire = new DataOutputStream(output);
        wire.write(new byte[] {'R', 'Q', 'O', '1'});
        wire.writeByte(command);
        if (command == 'E') {
            ArrayDataEncoder encoder = OpenRQ.newEncoder(object, parameters);
            wire.writeShort(packets.size());
            for (byte[] id : packets) {
                int sbn = id[0] & 255;
                int esi = esi(id);
                EncodingPacket packet = encoder.sourceBlock(sbn).encodingPacket(esi);
                ByteBuffer encoded = packet.symbols();
                require(encoded.remaining() <= symbolSize, "OpenRQ emitted oversized symbol");
                byte[] symbol = new byte[symbolSize];
                encoded.get(symbol, 0, encoded.remaining());
                // OpenRQ permits omitted final-source padding; this profile emits complete T bytes.
                wire.writeShort(4 + symbolSize);
                wire.write(id);
                wire.write(symbol);
            }
        }
        else {
            ArrayDataDecoder decoder = OpenRQ.newDecoder(parameters, 0);
            for (byte[] packet : packets) {
                int sbn = packet[0] & 255;
                Parsed<EncodingPacket> decoded = decoder.parsePacket(
                    sbn, esi(packet), Arrays.copyOfRange(packet, 4, packet.length), true);
                require(decoded.isValid(), "OpenRQ rejected packet: " + decoded.failureReason());
                decoder.sourceBlock(sbn).putEncodingPacket(decoded.value());
            }
            boolean complete = decoder.isDataDecoded();
            wire.writeByte(complete ? 1 : 0);
            if (complete) {
                byte[] data = decoder.dataArray();
                require(data.length == transferLength, "OpenRQ output length");
                wire.writeInt(data.length);
                wire.write(data);
            }
        }
        wire.flush();
        return output.toByteArray();
    }

    private static void validateId(byte[] packet, int blocks) {
        require((packet[0] & 255) < blocks, "SBN outside object");
        require(esi(packet) <= 4095, "bounded ESI");
    }

    private static int esi(byte[] packet) {
        return ((packet[1] & 255) << 16) | ((packet[2] & 255) << 8) | (packet[3] & 255);
    }

    private static int u8(ByteBuffer input) {
        require(input.hasRemaining(), "truncated request");
        return input.get() & 255;
    }

    private static int u16(ByteBuffer input) {
        require(input.remaining() >= 2, "truncated request");
        return input.getShort() & 65535;
    }

    private static byte[] read(ByteBuffer input, int length) {
        require(length >= 0 && length <= input.remaining(), "truncated request");
        byte[] data = new byte[length];
        input.get(data);
        return data;
    }

    private static void require(boolean condition, String reason) {
        if (!condition) throw new IllegalArgumentException(reason);
    }
}
