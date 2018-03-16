// Copyright (c) 2017-2018, Substratum LLC (https://substratum.net) and/or its affiliates. All rights reserved.
use regex::Regex;
use utils::index_of;
use http_packet_framer::PacketProgressState;
use http_packet_framer::ChunkExistenceState;
use http_packet_framer::ChunkProgressState;
use http_packet_framer::HttpPacketStartFinder;
use http_packet_framer::HttpFramerState;

const LONGEST_PREFIX_LEN: usize = 13;

pub struct HttpResponseStartFinder {}

impl HttpPacketStartFinder for HttpResponseStartFinder {
    fn seek_packet_start(&self, framer_state: &mut HttpFramerState) -> bool {
        if framer_state.packet_progress_state == PacketProgressState::SeekingPacketStart {
            match HttpResponseStartFinder::find_response_offset (&framer_state.data_so_far[..]) {
                Some(response_offset) => {
                    let clean_start_data = framer_state.data_so_far.split_off(response_offset);
                    framer_state.data_so_far = clean_start_data;
                    framer_state.packet_progress_state = PacketProgressState::SeekingBodyStart;
                    framer_state.content_length = 0;
                    framer_state.transfer_encoding_chunked = ChunkExistenceState::Standard;
                    framer_state.chunk_progress_state = ChunkProgressState::None;
                    framer_state.chunk_size = None;
                    framer_state.lines.clear ();
                    true
                },
                None => {
                    let index = if framer_state.data_so_far.len () > LONGEST_PREFIX_LEN
                        {framer_state.data_so_far.len () - LONGEST_PREFIX_LEN} else {0};
                    let remainder = framer_state.data_so_far.split_off (index);
                    framer_state.data_so_far = remainder;
                    false
                }
            }
        }
        else {
            false
        }
    }
}

impl HttpResponseStartFinder {
    fn find_response_offset (data_so_far: &[u8]) -> Option<usize> {
        let http_offset = match index_of (data_so_far, b"HTTP/") {
            Some (http_offset) => http_offset,
            None => return None
        };
        let possibility_u8 = &data_so_far[http_offset..(http_offset + LONGEST_PREFIX_LEN)];
        let possibility_cow = String::from_utf8_lossy (possibility_u8);
        let regex = Regex::new ("HTTP/1\\.[01] \\d\\d\\d ").expect ("Internal error");
        match regex.find (&possibility_cow) {
            Some (re_match) => Some (re_match.start () + http_offset),
            None => match HttpResponseStartFinder::find_response_offset (&data_so_far[(http_offset + 1)..]) {
                Some (next_match) => Some (next_match + http_offset + 1),
                None => None
            }
        }
    }
}

#[cfg (test)]
mod tests {
    use super::*;
    use http_packet_framer::ChunkProgressState;

    #[test]
    fn returns_none_if_no_http () {
        let data = b"there is no HTTP followed by a / here";

        let result = HttpResponseStartFinder::find_response_offset(&data[..]);

        assert_eq! (result, None);
    }

    #[test]
    fn returns_none_if_http_slash_but_no_immediate_status () {
        let data = b"Here we have HTTP/1.1 but no status until here: 200";

        let result = HttpResponseStartFinder::find_response_offset(&data[..]);

        assert_eq! (result, None);
    }

    #[test]
    fn returns_offset_if_header_exists () {
        let data = b"Now there's a full HTTP/1.1 235 first line";

        let result = HttpResponseStartFinder::find_response_offset(&data[..]);

        assert_eq! (result, Some ("Now there's a full ".len ()));
    }

    #[test]
    fn ignores_initial_incompletes () {
        let data = b"Here's a fake HTTP/1.1 20 followed by a real HTTP/1.0 200 OK\r\n\r\n";

        let result = HttpResponseStartFinder::find_response_offset(&data[..]);

        assert_eq! (result, Some ("Here's a fake HTTP/1.1 20 followed by a real ".len ()));
    }

    #[test]
    fn refuses_to_operate_in_state_other_than_seeking_request_start () {
        let mut framer_state = HttpFramerState {
            data_so_far: Vec::from("HTTP/1.1 499 Made-Up Error Code\r\n".as_bytes()),
            packet_progress_state: PacketProgressState::SeekingBodyStart,
            content_length: 100,
            transfer_encoding_chunked: ChunkExistenceState::ChunkedResponse,
            chunk_progress_state: ChunkProgressState::None,
            chunk_size: None,
            lines: vec! (vec! (), vec! ()),
        };
        let subject = HttpResponseStartFinder {};

        let result = subject.seek_packet_start(&mut framer_state);

        assert_eq!(result, false);
        assert_eq!(framer_state, HttpFramerState {
            data_so_far: Vec::from("HTTP/1.1 499 Made-Up Error Code\r\n".as_bytes()),
            packet_progress_state: PacketProgressState::SeekingBodyStart,
            content_length: 100,
            transfer_encoding_chunked: ChunkExistenceState::ChunkedResponse,
            chunk_progress_state: ChunkProgressState::None,
            chunk_size: None,
            lines: vec! (vec! (), vec! ()),
        });
    }

    #[test]
    fn throws_away_leading_garbage_except_for_last_thirteen_characters () {
        let mut framer_state = HttpFramerState {
            data_so_far: Vec::from("this is garbage".as_bytes()),
            packet_progress_state: PacketProgressState::SeekingPacketStart,
            content_length: 100,
            transfer_encoding_chunked: ChunkExistenceState::ChunkedResponse,
            chunk_progress_state: ChunkProgressState::None,
            chunk_size: None,
            lines: vec! (vec! (), vec! ()),
        };
        let subject = HttpResponseStartFinder {};

        let result = subject.seek_packet_start(&mut framer_state);

        assert_eq!(result, false);
        assert_eq!(framer_state, HttpFramerState {
            data_so_far: Vec::from("is is garbage".as_bytes()),
            packet_progress_state: PacketProgressState::SeekingPacketStart,
            content_length: 100,
            transfer_encoding_chunked: ChunkExistenceState::ChunkedResponse,
            chunk_progress_state: ChunkProgressState::None,
            chunk_size: None,
            lines: vec! (vec! (), vec! ()),
        });
    }

    #[test]
    fn frames_properly_in_the_presence_of_garbage () {
        let mut framer_state = HttpFramerState {
            data_so_far: Vec::from("this is garbageHTTP/1.1 499 Made-Up Error Code\r\n\
One-Header: value\r\n\
Another-Header: val".as_bytes()),
            packet_progress_state: PacketProgressState::SeekingPacketStart,
            content_length: 100,
            transfer_encoding_chunked: ChunkExistenceState::ChunkedResponse,
            chunk_progress_state: ChunkProgressState::SeekingEndOfFinalChunk,
            chunk_size: Some (200),
            lines: vec! (vec! (), vec! ()),
        };
        let subject = HttpResponseStartFinder {};

        let result = subject.seek_packet_start(&mut framer_state);

        assert_eq!(result, true);
        assert_eq!(framer_state, HttpFramerState {
            data_so_far: Vec::from("HTTP/1.1 499 Made-Up Error Code\r\n\
One-Header: value\r\n\
Another-Header: val".as_bytes()),
            packet_progress_state: PacketProgressState::SeekingBodyStart,
            content_length: 0,
            transfer_encoding_chunked: ChunkExistenceState::Standard,
            chunk_progress_state: ChunkProgressState::None,
            chunk_size: None,
            lines: vec! (),
        });
    }
}
