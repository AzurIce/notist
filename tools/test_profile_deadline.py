import unittest


class MeasurementContract(unittest.TestCase):
    def test_http_body_reader_preserves_pipeline_boundaries(self):
        from profile_serve_edits import read_body
        from io import BytesIO
        stream = BytesIO(b'3\r\nabc\r\n2\r\nde\r\n0\r\n\r\nnext')
        self.assertEqual(read_body(stream, {b'transfer-encoding': b'chunked'}), b'abcde')
        self.assertEqual(stream.read(), b'next')
        stream = BytesIO(b'abcnext')
        self.assertEqual(read_body(stream, {b'content-length': b'3'}), b'abc')
        self.assertEqual(stream.read(), b'next')

    def test_preview_normalization_only_removes_exactly_one_injected_script(self):
        from profile_serve_edits import without_preview_script
        for prefix in ['./', '../', '../../']:
            script = f'<script type="module" src="{prefix}_notist/serve/client.js?v=12"></script>'.encode()
            content = b'<body>kept<script src="app.js"></script>'
            self.assertEqual(without_preview_script(content + script + b'</body>'), content + b'</body>')
            with self.assertRaises(AssertionError):
                without_preview_script(script + script)
        with self.assertRaises(AssertionError):
            without_preview_script(b'no injection')

    def test_decimal_memory_threshold(self):
        self.assertGreater(300 * 1024 * 1024, 300_000_000)
        self.assertEqual(292968 * 1024, 299999232)
        self.assertGreater(292969 * 1024, 300_000_000)

    def test_deadline_epoch(self):
        import datetime
        tz = datetime.timezone(datetime.timedelta(hours=8))
        self.assertEqual(datetime.datetime.fromtimestamp(1793851200, tz).isoformat(), '2026-11-05T12:00:00+08:00')


if __name__ == '__main__':
    unittest.main()
