# Weather WMS Dashboard

A web-based dashboard to view and test your Weather WMS server using Leaflet maps.

## Features

✨ **Service Status Monitoring**
- Real-time WMS service status indicator
- Real-time WMTS service status indicator
- Auto-refresh every 30 seconds

📊 **Layer Management**
- Display all available WMS layers
- Show layer metadata (name, title, description)
- Display layer dimensions (TIME, etc.)
- Display layer extent information
- Query capability indicator

🗺️ **Interactive Map**
- OpenStreetMap base layer
- WMS layer overlay
- Support for multiple coordinate systems
- Responsive design

## Quick Start

### Prerequisites
- WMS API server running on `localhost:8080`
- Python 3.x (for the simple HTTP server)
- A modern web browser

### Option 1: Using Python (Recommended)

```bash
cd web
python3 server.py
```

Then open: **http://localhost:8000**

### Option 2: Using Python's built-in server

```bash
cd web
python3 -m http.server 8000
```

Then open: **http://localhost:8000**

### Option 3: Using Node.js

```bash
cd web
npx http-server
```

Then open: **http://localhost:8080** (or the port shown in terminal)

### Option 4: Using Docker

```bash
docker run -p 8000:80 -v $(pwd)/web:/usr/share/nginx/html nginx
```

Then open: **http://localhost:8000**

## File Structure

```
web/
├── index.html      # Main HTML page with Leaflet map
├── style.css       # Dashboard styling
├── app.js          # Application logic and WMS/WMTS integration
├── server.py       # Simple Python HTTP server with CORS
└── README.md       # This file
```

## How It Works

### Service Status

The dashboard automatically checks both WMS and WMTS service status:
- 🟢 **Green**: Service is online and responding
- 🔴 **Red**: Service is offline or not responding
- 🟡 **Yellow**: Status unknown

Status is checked on page load and every 30 seconds thereafter.

### Capabilities Parsing

The dashboard fetches and parses WMS GetCapabilities XML to extract:
- Layer names and titles
- Layer descriptions
- Available dimensions (e.g., TIME)
- Extent information
- Queryable flag

### Map Integration

When you select a layer:
1. Layer metadata is displayed in the sidebar
2. The WMS layer is added to the Leaflet map
3. The map shows the layer with OpenStreetMap as a base layer

## Configuration

### WMS Server URL

By default, the dashboard connects to `http://localhost:8080`

To change this, edit `app.js`:

```javascript
const API_BASE = 'http://your-wms-server:8080';
```

### Map Center and Zoom

Default: Center at [20, 0], Zoom level 3

To change, edit `app.js`:

```javascript
map = L.map('map').setView([latitude, longitude], zoomLevel);
```

## Troubleshooting

### "Failed to load layers"
- Ensure the WMS API server is running on localhost:8080
- Check browser console for CORS errors
- Try accessing the WMS capabilities directly: http://localhost:8080/wms?SERVICE=WMS&REQUEST=GetCapabilities

### CORS Errors
- The included `server.py` adds CORS headers automatically
- If using another server, ensure CORS is enabled for localhost:8080

### Layers not appearing on map
- Check the WMS API console for errors
- Verify the layer name is correct
- Ensure the layer has defined extent/bounds

## Browser Compatibility

- Chrome/Chromium 90+
- Firefox 88+
- Safari 14+
- Edge 90+

## Future Enhancements

Potential features to add:
- [ ] Time dimension slider (for temporal data)
- [ ] Style selector (if multiple styles available)
- [ ] Feature info popup (GetFeatureInfo)
- [ ] Layer transparency slider
- [ ] Coordinate display
- [ ] Multiple layer selection
- [ ] Custom extent bounds input
- [ ] Dark mode toggle
- [ ] Export map as image

## Technologies

- **Frontend**: HTML5, CSS3, JavaScript (ES6+)
- **Mapping**: Leaflet.js 1.9.4
- **Base Map**: OpenStreetMap
- **Server**: Python http.server or similar
- **WMS Server**: Custom Rust-based WMS API

## License

Same license as the Weather WMS project

## Contributing

To improve the dashboard:
1. Edit HTML/CSS/JS in the `web/` directory
2. Test in your browser
3. Submit improvements!

## Support

If you encounter issues:
1. Check the browser console (F12) for JavaScript errors
2. Check the WMS server logs
3. Verify the WMS server is responding to capabilities requests
4. Ensure CORS is properly configured
