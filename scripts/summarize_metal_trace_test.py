import io
import unittest

from summarize_metal_trace import Table, distribution, summarize, union_ns


def table(schema, columns, rows):
    return Table(io.StringIO('<trace-query-result><node><schema name="' + schema + '">' +
                            ''.join('<col><mnemonic>' + c + '</mnemonic></col>' for c in columns) +
                            '</schema>' + ''.join('<row>' + r + '</row>' for r in rows) +
                            '</node></trace-query-result>'), schema)


def gpu_row(start, duration, identity=3, process='<process ref="p"/>', depth=0):
    return (f'<start-time>{start}</start-time><duration>{duration}</duration>' + process +
            f'<metal-nesting-level>{depth}</metal-nesting-level><gpu-state>Active</gpu-state>' +
            f'<metal-command-buffer-id>{identity}</metal-command-buffer-id><gpu-channel-name>Vertex</gpu-channel-name>')


def fixtures(extra=()):
    gpu = table('metal-gpu-intervals', ['start', 'duration', 'process', 'event-depth', 'state', 'cmdbuffer-id', 'channel-name'], [
        gpu_row(10, 10, process='<process id="p"><pid id="pid">7</pid></process>'),
        gpu_row(15, 10, process='<process><pid ref="pid"/></process>'),
        gpu_row(12, 2, depth=1),  # Overlap and nested duplicate.
        gpu_row(30, 5, identity=4),  # No encoding label: explicit exclusion.
        gpu_row(1, 10, identity=5), gpu_row(15, 5, identity=5),  # Whole boundary-crossing buffer excluded.
        gpu_row(20, 100, process='<process><pid>99</pid></process>'),
        *extra,
    ])
    app = table('metal-application-intervals', ['process', 'event-type', 'cmdbuffer-id', 'event-label'], [
        '<process><pid>7</pid></process><metal-event-name>Encoding</metal-event-name>' +
        '<metal-command-buffer-id>3</metal-command-buffer-id>' +
        '<formatted-label><metal-object-label id="label">render</metal-object-label><uint64>1</uint64></formatted-label>',
        '<process><pid>99</pid></process><metal-event-name>Encoding</metal-event-name>' +
        '<metal-command-buffer-id>3</metal-command-buffer-id>' +
        '<formatted-label><metal-object-label>unrelated-secret</metal-object-label><uint64>1</uint64></formatted-label>',
    ])
    return gpu, app


class MetalTraceTests(unittest.TestCase):
    def test_process_filter_references_overlap_nested_and_boundary_rows(self):
        report = summarize(*fixtures(), 7, 10, 100)
        self.assertEqual(len(report['buffers']), 1)
        buffer = report['buffers'][0]
        self.assertEqual(buffer['label'], 'render')
        self.assertEqual(buffer['span_ms'], 15 / 1e6)
        self.assertEqual(buffer['active_union_ms'], 15 / 1e6)
        self.assertEqual(len(buffer['intervals']), 2)
        self.assertEqual(report['excluded'], {'non_top_level_active_rows': 1,
                                            'unmatched_buffers': 1, 'outside_or_crossing_window_buffers': 1})
        self.assertNotIn('unrelated-secret', str(report))

    def test_union_excludes_gaps_and_does_not_double_count_overlaps(self):
        self.assertEqual(union_ns([(30, 40), (10, 20), (15, 25), (11, 12)]), 25)
        self.assertEqual(union_ns([]), 0)
        with self.assertRaises(ValueError):
            union_ns([(3, 2)])

    def test_nearest_rank_distributions(self):
        result = distribution([5, 1, 2, 3, 4])
        self.assertEqual(result['p50_ms'], 3)
        self.assertEqual(result['p95_ms'], 5)
        self.assertEqual(result['samples_ms'], [1, 2, 3, 4, 5])
        for values in ([], [float('nan')], [-1]):
            with self.assertRaises(ValueError):
                distribution(values)

    def test_required_render_work_cannot_be_replaced_by_upload_only_work(self):
        summarize(*fixtures(), 7, 10, 100, ['render'])
        with self.assertRaisesRegex(ValueError, 'Missing required GPU'):
            summarize(*fixtures(), 7, 10, 100, ['egui_render'])

    def test_missing_referenced_and_numeric_target_data_fail(self):
        for extra in ([gpu_row(-1, 1)], [gpu_row(10, 1, process='<process ref="missing"/>')],
                      [gpu_row(10, 1, process='<process id="cycle" ref="cycle"/>')],
                      ['<duration>1</duration>']):
            with self.assertRaises(ValueError):
                summarize(*fixtures(extra), 7, 10, 100)
        for pid, start, end in ((0, 0, 1), (7, 2, 1), (7, -1, 1), (8, 10, 100)):
            with self.assertRaises(ValueError):
                summarize(*fixtures(), pid, start, end)

    def test_schema_and_duplicate_identity_validation(self):
        for xml in ('<trace-query-result/>', '<trace-query-result><node/></trace-query-result>',
                    '<trace-query-result><node><schema name="wrong"/></node></trace-query-result>',
                    '<trace-query-result><node><schema name="gpu"/><row><v id="1"/><v id="1"/></row></node></trace-query-result>'):
            with self.assertRaises(ValueError):
                Table(io.StringIO(xml), 'gpu')
        with self.assertRaises(ValueError):
            table('gpu', ['process', 'process'], [])


if __name__ == '__main__':
    unittest.main()
