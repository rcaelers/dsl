import sigrokdecode as srd


class Decoder(srd.Decoder):
    api_version = 3
    id = 'test_logic'
    name = 'Test Logic'
    longname = 'Test Logic Decoder'
    desc = 'Checked-in graph-node test decoder.'
    license = 'mit'
    inputs = ['logic']
    outputs = ['test_logic']
    tags = ['Test']
    channels = (
        {'id': 'mosi', 'name': 'MOSI', 'desc': 'Test input'},
    )
    optional_channels = (
        {'id': 'cs', 'name': 'CS', 'desc': 'Test select'},
    )
    options = ()
    annotations = ()
    annotation_rows = ()
    binary = ()

    def metadata(self, key, value):
        self.samplerate = value

    def start(self):
        self.output = self.register(srd.OUTPUT_PYTHON, proto_id='test_logic')
