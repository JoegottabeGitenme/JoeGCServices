#SSHCZO -- Digital Elevation Model (DEM), GIS/Map Data, Land Cover, LiDAR, Soil Survey -- Shale Hills -- (2010)
------
##OVERVIEW
###Description/Abstract
High-resolution Lidar data (average 10 points/m2 with 2-4 cm vertical accuracy) were collected for the Susquehanna Shale Hills CZO (Area = 169.80901 km2) during leaf-on (7/14/2010-7/16/2010) and full leaf-off (snow clear) (12/3/2010-12/9/2010). Data acquisition, ground-truthing, vegetation surveys and processing were funded and coordinated by NSF Award EAR-0922307 (PI. Qinghua Guo). Data was collected with the Gemini 06SEN/CON195 and digitizer 08DIG017 system installed on the Cessna 337 tail number N337P. Total points: 2,840,000,000 pts. Area: Area = 169 km2. Shot density: 13.54 points/m2. Survey report, with details about data processing: http://opentopo.sdsc.edu/metadata/2010_NCALM_CZO_Project_Report.pdf. All files are in ArcGRID format.

###Dataset DOI
http://dx.doi.org/10.5069/G9VM496T

###Creator/Author
Guo, Qinghua

###CZOs
Shale Hills

###Contact
Dr. Qinghua Guo. University of California-Merced. P.O. Box 2039. Merced, CA 95344. e-mail: qguo@ucmerced.edu. phone: (209) 228-2911.

###Subtitle
Shaver's Creek Watershed

  
<br /><br />
  ------
##SUBJECTS
###Disciplines
GIS / Remote Sensing

###Topics
Digital Elevation Model (DEM)|GIS/Map Data|Land Cover|LiDAR|Soil Survey

###Keywords
lidar|topography|digital elevation model|hillshade|density

###Variables
Filtered (bare earth) DEM|Filtered (bare earth) hillshade|Unfiltered (first return) DEM|Unfiltered (first return) hillshade

###Variables ODM2
Digital elevation model|Lidar

  
<br /><br />
  ------
##TEMPORAL
###Date Start
2010-07-14

###Date End
2010-12-09

  
<br /><br />
  ------
##SPATIAL
###Field Areas
Susquehanna Shale Hills Critical Zone Observatory

###Location
Shale Hills

###North latitude
40.7319234451

###South latitude
40.5603495059

###West longitude
-78.0857145194

###East longitude
-77.8470628291

  
<br /><br />
  ------
##REFERENCE
###Citation
LiDAR data acquisition and processing were completed by the National Center for Airborne Laser Mapping (NCALM), funded by the National Science Foundation Award EAR-0922307, and coordinated by Qinghua Guo for the Susquehanna Shale Hills Critical Zone Observatory funded by the National Science Foundation Award EAR-0725019. http://dx.doi.org/10.5069/G9VM496T

###CZO ID
2570

###External Links
<a href='http://www.dcnr.state.pa.us/topogeo/pamap/lidar/index.htm' target='_blank'>PAMAP LiDAR</a> | 

###Award Grant Numbers
<a href='http://www.nsf.gov/awardsearch/showAward?AWD_ID=0922307' target='_blank'>National Science Foundation - EAR-0922307</a>

  
<br /><br />
  ------
##COMMENTS
###Comments
Additional LiDAR data for the Commonwealth of Pennsylvania are available through the PA Department of Conservation and Natural Resources' PAMAP program, at the link provided in the sidebar.



Shaver's Creek and Shale Hills watershed boundary DEM files contain 0.5 meter resolution DEM from LiDAR collected in February 2011. Grid cell dimension = 0.5 x 0.5 m, Projection = ' +proj=utm +zone=18 +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs' (UTM 18N), Gaussian Filter with square 4 x 4 m smoothing window applied to data to produce DEM. Boundary Delineation Algorithms consisted of three steps: 1) Calculate Upslope Contributing Area of catchment with DEM using multiple algorithms; 2) Model channel network using the DEM and upslope contributing area map(s); 3) Input channel network and DEM into a basin delineation algorithm. These steps were performed in SAGA GIS, which uses same algoritms as in TauDEM (ArcMap extension).

