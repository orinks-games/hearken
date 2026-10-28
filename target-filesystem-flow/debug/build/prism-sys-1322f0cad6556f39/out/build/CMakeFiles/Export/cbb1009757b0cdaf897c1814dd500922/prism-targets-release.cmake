#----------------------------------------------------------------
# Generated CMake target import file for configuration "Release".
#----------------------------------------------------------------

# Commands may need to know the format version.
set(CMAKE_IMPORT_FILE_VERSION 1)

# Import target "prism::_prism_lib" for configuration "Release"
set_property(TARGET prism::_prism_lib APPEND PROPERTY IMPORTED_CONFIGURATIONS RELEASE)
set_target_properties(prism::_prism_lib PROPERTIES
  IMPORTED_LINK_INTERFACE_LANGUAGES_RELEASE "C;CXX"
  IMPORTED_LOCATION_RELEASE "${_IMPORT_PREFIX}/lib/libprism.a"
  )

list(APPEND _cmake_import_check_targets prism::_prism_lib )
list(APPEND _cmake_import_check_files_for_prism::_prism_lib "${_IMPORT_PREFIX}/lib/libprism.a" )

# Commands beyond this point should not need to know the version.
set(CMAKE_IMPORT_FILE_VERSION)
